use crate::{
    api::files::FileResponse,
    db::models::{FileRecord, ResumableSession},
    error::AppError,
    processor::analyze_and_process,
    search::meili::MeiliDocument,
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateResumablePayload {
    pub filename: String,
    pub total_size: i64,
    pub mime_type: Option<String>,
    pub app_id: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ResumableSessionResponse {
    pub id: String,
    pub filename: String,
    pub total_size: i64,
    pub current_offset: i64,
    pub upload_url: String,
    pub expires_at: String,
}

pub async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateResumablePayload>,
) -> Result<impl IntoResponse, AppError> {
    let session_id = Uuid::new_v4().to_string();
    let app_id = payload
        .app_id
        .or_else(|| {
            headers
                .get("x-app-id")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "general".to_string());

    let mime_type = payload.mime_type.unwrap_or_else(|| {
        mime_guess::from_path(&payload.filename)
            .first_or_octet_stream()
            .to_string()
    });

    let max_bytes = (state.config.max_upload_size_mb * 1024 * 1024) as i64;
    if payload.total_size > max_bytes {
        return Err(AppError::BadRequest(format!(
            "Total file size {} exceeds maximum permitted limit {} MB",
            payload.total_size, state.config.max_upload_size_mb
        )));
    }

    let temp_dir = PathBuf::from(&state.config.storage_temp_dir);
    let _ = fs::create_dir_all(&temp_dir).await;
    let temp_file_path = temp_dir.join(format!("{}.part", session_id));

    // Initialize empty temporary part file
    let _ = fs::File::create(&temp_file_path).await.map_err(|e| {
        AppError::Storage(format!("Failed to create temporary session file: {}", e))
    })?;

    let now = Utc::now();
    let expires_at = (now + Duration::hours(24)).to_rfc3339();
    let created_at = now.to_rfc3339();

    let tags_json = payload.tags.map(|t| json!(t).to_string());

    let session = ResumableSession {
        id: session_id.clone(),
        app_id,
        filename: payload.filename.clone(),
        mime_type,
        total_size: payload.total_size,
        current_offset: 0,
        temp_file_path: temp_file_path.to_string_lossy().to_string(),
        metadata_json: tags_json,
        created_at,
        expires_at: expires_at.clone(),
    };

    sqlx::query(
        r#"
        INSERT INTO resumable_sessions (
            id, app_id, filename, mime_type, total_size,
            current_offset, temp_file_path, metadata_json, created_at, expires_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(&session.id)
    .bind(&session.app_id)
    .bind(&session.filename)
    .bind(&session.mime_type)
    .bind(session.total_size)
    .bind(session.current_offset)
    .bind(&session.temp_file_path)
    .bind(&session.metadata_json)
    .bind(&session.created_at)
    .bind(&session.expires_at)
    .execute(&state.db)
    .await
    .map_err(AppError::Database)?;

    let upload_url = format!("/api/v1/files/resumable/{}", session_id);
    let resp = ResumableSessionResponse {
        id: session_id,
        filename: payload.filename,
        total_size: payload.total_size,
        current_offset: 0,
        upload_url,
        expires_at,
    };

    Ok((
        StatusCode::CREATED,
        [
            (header::LOCATION, format!("/api/v1/files/resumable/{}", resp.id)),
            (header::HeaderName::from_static("upload-offset"), "0".to_string()),
        ],
        Json(json!({ "success": true, "data": resp })),
    ))
}

pub async fn get_session_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Response, AppError> {
    let session: Option<ResumableSession> =
        sqlx::query_as("SELECT * FROM resumable_sessions WHERE id = ?1")
            .bind(&session_id)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::Database)?;

    let session = session.ok_or_else(|| {
        AppError::NotFound(format!("Resumable upload session {} not found", session_id))
    })?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::HeaderName::from_static("upload-offset"),
            session.current_offset.to_string(),
        )
        .header(
            header::HeaderName::from_static("upload-length"),
            session.total_size.to_string(),
        )
        .body(axum::body::Body::empty())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed building response: {}", e)))?;

    Ok(response)
}

pub async fn append_chunk(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    chunk_bytes: Bytes,
) -> Result<Response, AppError> {
    let session: Option<ResumableSession> =
        sqlx::query_as("SELECT * FROM resumable_sessions WHERE id = ?1")
            .bind(&session_id)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::Database)?;

    let mut session = session.ok_or_else(|| {
        AppError::NotFound(format!("Resumable upload session {} not found", session_id))
    })?;

    // Check Upload-Offset header if provided
    if let Some(offset_header) = headers
        .get("upload-offset")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        if offset_header != session.current_offset {
            return Err(AppError::BadRequest(format!(
                "Upload-Offset mismatch: expected {}, received {}",
                session.current_offset, offset_header
            )));
        }
    }

    let chunk_len = chunk_bytes.len() as i64;
    let new_offset = session.current_offset + chunk_len;

    if new_offset > session.total_size {
        return Err(AppError::BadRequest(format!(
            "Appended chunk exceeds declared total size {}",
            session.total_size
        )));
    }

    // Append chunk to temp file
    let mut file = OpenOptions::new()
        .write(true)
        .append(true)
        .open(&session.temp_file_path)
        .await
        .map_err(|e| AppError::Storage(format!("Failed to open temp chunk file: {}", e)))?;

    file.write_all(&chunk_bytes)
        .await
        .map_err(|e| AppError::Storage(format!("Failed appending chunk bytes: {}", e)))?;

    session.current_offset = new_offset;

    // Check if upload is completed
    if session.current_offset == session.total_size {
        // Finalize upload
        let full_bytes = fs::read(&session.temp_file_path)
            .await
            .map_err(|e| AppError::Storage(format!("Failed reading completed file: {}", e)))?;

        // Calculate SHA-256
        let mut hasher = Sha256::new();
        hasher.update(&full_bytes);
        let sha256 = hex::encode(hasher.finalize());

        let file_id = Uuid::new_v4().to_string();
        let storage_path = format!("raw/{}/{}", file_id, session.filename);

        // Put to storage
        state
            .storage
            .put_object(&storage_path, &full_bytes, &session.mime_type)
            .await
            .map_err(|e| AppError::Storage(format!("Failed persisting object: {}", e)))?;

        // Process media
        let processed = analyze_and_process(
            &session.mime_type,
            &full_bytes,
            state.config.thumbnail_max_width,
            state.config.thumbnail_max_height,
        );

        let mut has_thumbnail = false;
        let mut thumbnail_path = None;
        if let Some(ref thumb_bytes) = processed.thumbnail_bytes {
            let t_path = format!("thumbnails/{}.webp", file_id);
            if state
                .storage
                .put_object(&t_path, thumb_bytes, "image/webp")
                .await
                .is_ok()
            {
                has_thumbnail = true;
                thumbnail_path = Some(t_path);
            }
        }

        let now = Utc::now().to_rfc3339();
        let record = FileRecord {
            id: file_id.clone(),
            app_id: session.app_id.clone(),
            filename: session.filename.clone(),
            mime_type: session.mime_type.clone(),
            file_size: session.total_size,
            sha256: sha256.clone(),
            storage_backend: state.storage.name().to_string(),
            storage_path,
            has_thumbnail,
            thumbnail_path,
            extracted_text: processed.extracted_text.clone(),
            tags: None,
            metadata_json: session.metadata_json.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        sqlx::query(
            r#"
            INSERT INTO files (
                id, app_id, filename, mime_type, file_size, sha256,
                storage_backend, storage_path, has_thumbnail, thumbnail_path,
                extracted_text, tags, metadata_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
        )
        .bind(&record.id)
        .bind(&record.app_id)
        .bind(&record.filename)
        .bind(&record.mime_type)
        .bind(record.file_size)
        .bind(&record.sha256)
        .bind(&record.storage_backend)
        .bind(&record.storage_path)
        .bind(record.has_thumbnail)
        .bind(&record.thumbnail_path)
        .bind(&record.extracted_text)
        .bind(&record.tags)
        .bind(&record.metadata_json)
        .bind(&record.created_at)
        .bind(&record.updated_at)
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;

        // Clean up temp file & session
        let _ = fs::remove_file(&session.temp_file_path).await;
        let _ = sqlx::query("DELETE FROM resumable_sessions WHERE id = ?1")
            .bind(&session_id)
            .execute(&state.db)
            .await;

        // Index to Meilisearch
        let search_client = state.search.clone();
        let meili_doc = MeiliDocument {
            id: record.id.clone(),
            app_id: record.app_id.clone(),
            filename: record.filename.clone(),
            mime_type: record.mime_type.clone(),
            file_size: record.file_size,
            sha256: record.sha256.clone(),
            has_thumbnail: record.has_thumbnail,
            tags: Vec::new(),
            extracted_text: record.extracted_text.clone(),
            metadata_json: record.metadata_json.clone(),
            created_at: record.created_at.clone(),
        };

        tokio::spawn(async move {
            let _ = search_client.index_document(&meili_doc).await;
        });

        let file_resp = FileResponse::from_record(&record);
        let body = serde_json::to_string(&json!({
            "success": true,
            "status": "completed",
            "data": file_resp
        }))
        .unwrap_or_default();

        let response = Response::builder()
            .status(StatusCode::CREATED)
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::HeaderName::from_static("upload-offset"),
                session.total_size.to_string(),
            )
            .body(axum::body::Body::from(body))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed building response: {}", e)))?;

        return Ok(response);
    }

    // Update session current offset
    sqlx::query("UPDATE resumable_sessions SET current_offset = ?1 WHERE id = ?2")
        .bind(session.current_offset)
        .bind(&session_id)
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;

    let response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(
            header::HeaderName::from_static("upload-offset"),
            session.current_offset.to_string(),
        )
        .body(axum::body::Body::empty())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed building response: {}", e)))?;

    Ok(response)
}

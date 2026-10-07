use crate::{
    db::models::FileRecord,
    error::AppError,
    processor::analyze_and_process,
    search::meili::MeiliDocument,
    state::AppState,
};
use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListFilesQuery {
    pub app_id: Option<String>,
    pub mime_type: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct FileResponse {
    pub id: String,
    pub app_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    pub sha256: String,
    pub has_thumbnail: bool,
    pub download_url: String,
    pub thumbnail_url: Option<String>,
    pub tags: Vec<String>,
    pub created_at: String,
}

impl FileResponse {
    pub fn from_record(r: &FileRecord) -> Self {
        let tags = r
            .tags
            .as_deref()
            .map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let thumbnail_url = if r.has_thumbnail {
            Some(format!("/api/v1/files/{}/thumbnail", r.id))
        } else {
            None
        };

        Self {
            id: r.id.clone(),
            app_id: r.app_id.clone(),
            filename: r.filename.clone(),
            mime_type: r.mime_type.clone(),
            file_size: r.file_size,
            sha256: r.sha256.clone(),
            has_thumbnail: r.has_thumbnail,
            download_url: format!("/api/v1/files/{}/raw", r.id),
            thumbnail_url,
            tags,
            created_at: r.created_at.clone(),
        }
    }
}

pub async fn upload_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let mut app_id = headers
        .get("x-app-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("general")
        .to_string();

    let mut filename = "unnamed.bin".to_string();
    let mut file_bytes = Vec::new();
    let mut tags = Vec::new();
    let mut metadata_json: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed reading multipart stream: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();

        if name == "file" {
            if let Some(orig_name) = field.file_name() {
                filename = orig_name.to_string();
            }
            file_bytes = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("Failed reading file payload: {}", e)))?
                .to_vec();
        } else if name == "app_id" {
            if let Ok(val) = field.text().await {
                if !val.trim().is_empty() {
                    app_id = val.trim().to_string();
                }
            }
        } else if name == "tags" {
            if let Ok(val) = field.text().await {
                tags = val
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        } else if name == "metadata" {
            if let Ok(val) = field.text().await {
                metadata_json = Some(val);
            }
        }
    }

    if file_bytes.is_empty() {
        return Err(AppError::BadRequest("No file was uploaded or file is empty".to_string()));
    }

    let file_size = file_bytes.len() as i64;
    let max_bytes = (state.config.max_upload_size_mb * 1024 * 1024) as i64;
    if file_size > max_bytes {
        return Err(AppError::BadRequest(format!(
            "File size {} exceeds maximum permitted limit {} MB",
            file_size, state.config.max_upload_size_mb
        )));
    }

    // MIME type deduction
    let mime_type = mime_guess::from_path(&filename)
        .first_or_octet_stream()
        .to_string();

    // Calculate SHA-256
    let mut hasher = Sha256::new();
    hasher.update(&file_bytes);
    let sha256 = hex::encode(hasher.finalize());

    let file_id = Uuid::new_v4().to_string();
    let storage_raw_path = format!("raw/{}/{}", file_id, filename);

    // Persist raw file
    state
        .storage
        .put_object(&storage_raw_path, &file_bytes, &mime_type)
        .await
        .map_err(|e| AppError::Storage(format!("Failed saving file to storage: {}", e)))?;

    // Process media (Thumbnail generation / PDF text extraction)
    let processed = analyze_and_process(
        &mime_type,
        &file_bytes,
        state.config.thumbnail_max_width,
        state.config.thumbnail_max_height,
    );

    let mut has_thumbnail = false;
    let mut thumbnail_path: Option<String> = None;

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
    let tags_str = if tags.is_empty() {
        None
    } else {
        Some(tags.join(","))
    };

    // Save record to database
    let record = FileRecord {
        id: file_id.clone(),
        app_id: app_id.clone(),
        filename: filename.clone(),
        mime_type: mime_type.clone(),
        file_size,
        sha256: sha256.clone(),
        storage_backend: state.storage.name().to_string(),
        storage_path: storage_raw_path,
        has_thumbnail,
        thumbnail_path,
        extracted_text: processed.extracted_text.clone(),
        tags: tags_str,
        metadata_json,
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

    // Index to Meilisearch in background task
    let search_client = state.search.clone();
    let meili_doc = MeiliDocument {
        id: record.id.clone(),
        app_id: record.app_id.clone(),
        filename: record.filename.clone(),
        mime_type: record.mime_type.clone(),
        file_size: record.file_size,
        sha256: record.sha256.clone(),
        has_thumbnail: record.has_thumbnail,
        tags,
        extracted_text: record.extracted_text.clone(),
        metadata_json: record.metadata_json.clone(),
        created_at: record.created_at.clone(),
    };

    tokio::spawn(async move {
        if let Err(e) = search_client.index_document(&meili_doc).await {
            tracing::warn!("Async Meilisearch index error: {}", e);
        }
    });

    let resp = FileResponse::from_record(&record);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "success": true,
            "data": resp
        })),
    ))
}

pub async fn get_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let record: Option<FileRecord> = sqlx::query_as("SELECT * FROM files WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(AppError::Database)?;

    let record = record.ok_or_else(|| AppError::NotFound(format!("File {} not found", id)))?;
    let resp = FileResponse::from_record(&record);

    Ok(Json(json!({
        "success": true,
        "data": resp
    })))
}

pub async fn download_raw(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let record: Option<FileRecord> = sqlx::query_as("SELECT * FROM files WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(AppError::Database)?;

    let record = record.ok_or_else(|| AppError::NotFound(format!("File {} not found", id)))?;

    let bytes = state
        .storage
        .get_object(&record.storage_path)
        .await
        .map_err(|e| AppError::Storage(format!("Failed reading file payload: {}", e)))?;

    let disposition = format!("inline; filename=\"{}\"", record.filename);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, record.mime_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed creating response: {}", e)))?;

    Ok(response)
}

pub async fn download_thumbnail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let record: Option<FileRecord> = sqlx::query_as("SELECT * FROM files WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(AppError::Database)?;

    let record = record.ok_or_else(|| AppError::NotFound(format!("File {} not found", id)))?;

    let thumb_path = record
        .thumbnail_path
        .ok_or_else(|| AppError::NotFound("File has no thumbnail".to_string()))?;

    let bytes = state
        .storage
        .get_object(&thumb_path)
        .await
        .map_err(|e| AppError::Storage(format!("Failed reading thumbnail: {}", e)))?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/webp")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed creating response: {}", e)))?;

    Ok(response)
}

pub async fn delete_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let record: Option<FileRecord> = sqlx::query_as("SELECT * FROM files WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(AppError::Database)?;

    let record = record.ok_or_else(|| AppError::NotFound(format!("File {} not found", id)))?;

    // Delete raw storage
    let _ = state.storage.delete_object(&record.storage_path).await;

    // Delete thumbnail if present
    if let Some(ref t_path) = record.thumbnail_path {
        let _ = state.storage.delete_object(t_path).await;
    }

    // Delete database record
    sqlx::query("DELETE FROM files WHERE id = ?1")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;

    // Delete from Meilisearch
    let search_client = state.search.clone();
    let doc_id = id.clone();
    tokio::spawn(async move {
        let _ = search_client.delete_document(&doc_id).await;
    });

    Ok(Json(json!({
        "success": true,
        "message": format!("File {} deleted successfully", id)
    })))
}

pub async fn list_files(
    State(state): State<AppState>,
    Query(params): Query<ListFilesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    let mut query_builder = sqlx::QueryBuilder::new("SELECT * FROM files WHERE 1=1 ");

    if let Some(ref app) = params.app_id {
        query_builder.push(" AND app_id = ");
        query_builder.push_bind(app);
    }

    if let Some(ref mime) = params.mime_type {
        if mime.ends_with('*') {
            let prefix = format!("{}%", mime.trim_end_matches('*'));
            query_builder.push(" AND mime_type LIKE ");
            query_builder.push_bind(prefix);
        } else {
            query_builder.push(" AND mime_type = ");
            query_builder.push_bind(mime);
        }
    }

    query_builder.push(" ORDER BY created_at DESC LIMIT ");
    query_builder.push_bind(limit);
    query_builder.push(" OFFSET ");
    query_builder.push_bind(offset);

    let records: Vec<FileRecord> = query_builder
        .build_query_as()
        .fetch_all(&state.db)
        .await
        .map_err(AppError::Database)?;

    let items: Vec<FileResponse> = records.iter().map(FileResponse::from_record).collect();

    Ok(Json(json!({
        "success": true,
        "data": {
            "items": items,
            "count": items.len(),
            "limit": limit,
            "offset": offset
        }
    })))
}

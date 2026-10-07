use crate::{
    api::files::FileResponse,
    error::AppError,
    services::ChunkResult,
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

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

    let session = state
        .media
        .create_resumable_session(
            payload.filename,
            payload.total_size,
            mime_type,
            app_id,
            payload.tags,
        )
        .await?;

    let upload_url = format!("/api/v1/files/resumable/{}", session.id);
    let resp = ResumableSessionResponse {
        id: session.id.clone(),
        filename: session.filename,
        total_size: session.total_size,
        current_offset: session.current_offset,
        upload_url,
        expires_at: session.expires_at,
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
    let session = state.media.get_resumable_session(&session_id).await?;

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
    let offset_header = headers
        .get("upload-offset")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok());

    let result = state
        .media
        .append_resumable_chunk(&session_id, &chunk_bytes, offset_header)
        .await?;

    match result {
        ChunkResult::Completed { record } => {
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
                    record.file_size.to_string(),
                )
                .body(axum::body::Body::from(body))
                .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed building response: {}", e)))?;

            Ok(response)
        }
        ChunkResult::Appended { current_offset } => {
            let response = Response::builder()
                .status(StatusCode::NO_CONTENT)
                .header(
                    header::HeaderName::from_static("upload-offset"),
                    current_offset.to_string(),
                )
                .body(axum::body::Body::empty())
                .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed building response: {}", e)))?;

            Ok(response)
        }
    }
}

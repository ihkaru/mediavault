use crate::{
    db::models::FileRecord,
    error::AppError,
    services::NewFileInput,
    state::AppState,
};
use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

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

    let input = NewFileInput {
        app_id,
        filename,
        file_bytes,
        tags,
        metadata_json,
    };

    let record = state.media.save_file(input).await?;
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
    let record = state.media.get_file(&id).await?;
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
    let record = state.media.get_file(&id).await?;
    let bytes = state.media.get_raw_bytes(&record.storage_path).await?;
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
    let record = state.media.get_file(&id).await?;
    let thumb_path = record
        .thumbnail_path
        .ok_or_else(|| AppError::NotFound("File has no thumbnail".to_string()))?;

    let bytes = state.media.get_raw_bytes(&thumb_path).await?;

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
    state.media.delete_file(&id).await?;

    Ok(Json(json!({
        "success": true,
        "message": format!("File {} deleted successfully", id)
    })))
}

pub async fn list_files(
    State(state): State<AppState>,
    Query(params): Query<ListFilesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = params.limit.unwrap_or(20);
    let offset = params.offset.unwrap_or(0);

    let records = state
        .media
        .list_files(
            params.app_id.as_deref(),
            params.mime_type.as_deref(),
            limit,
            offset,
        )
        .await?;

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

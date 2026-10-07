use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum AppError {
    #[error("Authentication failed: {0}")]
    Unauthorized(String),

    #[error("File not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Processing error: {0}")]
    Processing(String),

    #[error("Search index error: {0}")]
    Search(String),

    #[error("Internal server error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg),
            AppError::Storage(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "STORAGE_ERROR", msg),
            AppError::Database(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                err.to_string(),
            ),
            AppError::Processing(msg) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "PROCESSING_ERROR", msg)
            }
            AppError::Search(msg) => (StatusCode::BAD_GATEWAY, "SEARCH_ENGINE_ERROR", msg),
            AppError::Internal(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                err.to_string(),
            ),
        };

        let body = Json(json!({
            "success": false,
            "error": {
                "code": error_code,
                "message": message
            }
        }));

        (status, body).into_response
()
    }
}

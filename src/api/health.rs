use crate::{error::AppError, state::AppState};
use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;

pub async fn health_check(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    // 1. Check DB connectivity
    let db_ok = sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .is_ok();

    // 2. Check Meilisearch connectivity
    let meili_ok = state.search.is_healthy().await;

    let all_healthy = db_ok && (!state.search.is_enabled() || meili_ok);

    Ok(Json(json!({
        "status": if all_healthy { "healthy" } else { "degraded" },
        "service": "mediavault",
        "version": env!("CARGO_PKG_VERSION"),
        "storage_backend": state.storage.name(),
        "database": if db_ok { "connected" } else { "error" },
        "meilisearch": {
            "enabled": state.search.is_enabled(),
            "status": if meili_ok { "connected" } else { "unreachable" }
        }
    })))
}

pub async fn metrics(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let stats: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(file_size), 0) FROM files"
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or((0, 0));

    let total_files = stats.0;
    let total_bytes = stats.1;

    Ok(Json(json!({
        "total_files": total_files,
        "total_bytes": total_bytes,
        "storage_engine": state.storage.name(),
        "meili_enabled": state.search.is_enabled()
    })))
}

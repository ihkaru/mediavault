use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct SearchQueryParams {
    pub q: Option<String>,
    pub app_id: Option<String>,
    pub mime_type: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn search_files(
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> Result<impl IntoResponse, AppError> {
    let query_str = params.q.unwrap_or_default();
    let limit = params.limit.unwrap_or(20);
    let offset = params.offset.unwrap_or(0);

    let result = state
        .media
        .search_files(
            &query_str,
            params.app_id.as_deref(),
            params.mime_type.as_deref(),
            limit,
            offset,
        )
        .await?;

    Ok(Json(json!({
        "success": true,
        "engine": result.engine,
        "data": {
            "hits": result.hits,
            "estimated_total_hits": result.estimated_total_hits,
            "processing_time_ms": result.processing_time_ms,
            "query": result.query,
            "limit": result.limit,
            "offset": result.offset
        }
    })))
}

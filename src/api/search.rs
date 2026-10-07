use crate::{
    db::models::{FileRecord, SearchHit},
    error::AppError,
    state::AppState,
};
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
    let query_str = params.q.unwrap_or_default().trim().to_string();
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0);

    // 1. Try Meilisearch if enabled
    if state.search.is_enabled() {
        if let Ok(resp) = state
            .search
            .search(
                &query_str,
                params.app_id.as_deref(),
                params.mime_type.as_deref(),
                limit,
                offset,
            )
            .await
        {
            let hits: Vec<SearchHit> = resp
                .hits
                .into_iter()
                .map(|doc| {
                    let thumbnail_url = if doc.has_thumbnail {
                        Some(format!("/api/v1/files/{}/thumbnail", doc.id))
                    } else {
                        None
                    };

                    let snippet = doc.extracted_text.as_deref().map(|t| {
                        if t.chars().count() > 160 {
                            format!("{}...", t.chars().take(160).collect::<String>())
                        } else {
                            t.to_string()
                        }
                    });

                    SearchHit {
                        id: doc.id.clone(),
                        app_id: doc.app_id,
                        filename: doc.filename,
                        mime_type: doc.mime_type,
                        file_size: doc.file_size,
                        sha256: doc.sha256,
                        has_thumbnail: doc.has_thumbnail,
                        thumbnail_url,
                        download_url: format!("/api/v1/files/{}/raw", doc.id),
                        tags: doc.tags,
                        snippet,
                        created_at: doc.created_at,
                    }
                })
                .collect();

            return Ok(Json(json!({
                "success": true,
                "engine": "meilisearch",
                "data": {
                    "hits": hits,
                    "estimated_total_hits": resp.estimated_total_hits.unwrap_or(hits.len()),
                    "processing_time_ms": resp.processing_time_ms.unwrap_or(0),
                    "query": query_str,
                    "limit": limit,
                    "offset": offset
                }
            })));
        }
    }

    // 2. Database Fallback (when Meilisearch is unreachable or starting up)
    let start_time = std::time::Instant::now();
    let mut builder = sqlx::QueryBuilder::new("SELECT * FROM files WHERE 1=1 ");

    if !query_str.is_empty() {
        let pattern = format!("%{}%", query_str);
        builder.push(" AND (filename LIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" OR tags LIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" OR extracted_text LIKE ");
        builder.push_bind(pattern);
        builder.push(")");
    }

    if let Some(ref app) = params.app_id {
        builder.push(" AND app_id = ");
        builder.push_bind(app);
    }

    if let Some(ref mime) = params.mime_type {
        if mime.ends_with('*') {
            let prefix = format!("{}%", mime.trim_end_matches('*'));
            builder.push(" AND mime_type LIKE ");
            builder.push_bind(prefix);
        } else {
            builder.push(" AND mime_type = ");
            builder.push_bind(mime);
        }
    }

    builder.push(" ORDER BY created_at DESC LIMIT ");
    builder.push_bind(limit as i64);
    builder.push(" OFFSET ");
    builder.push_bind(offset as i64);

    let records: Vec<FileRecord> = builder
        .build_query_as()
        .fetch_all(&state.db)
        .await
        .map_err(AppError::Database)?;

    let hits: Vec<SearchHit> = records
        .into_iter()
        .map(|r| {
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

            let snippet = r.extracted_text.as_deref().map(|t| {
                if t.chars().count() > 160 {
                    format!("{}...", t.chars().take(160).collect::<String>())
                } else {
                    t.to_string()
                }
            });

            SearchHit {
                id: r.id.clone(),
                app_id: r.app_id,
                filename: r.filename,
                mime_type: r.mime_type,
                file_size: r.file_size,
                sha256: r.sha256,
                has_thumbnail: r.has_thumbnail,
                thumbnail_url,
                download_url: format!("/api/v1/files/{}/raw", r.id),
                tags,
                snippet,
                created_at: r.created_at,
            }
        })
        .collect();

    let duration_ms = start_time.elapsed().as_millis() as u64;

    Ok(Json(json!({
        "success": true,
        "engine": "database_fallback",
        "data": {
            "hits": hits,
            "estimated_total_hits": hits.len(),
            "processing_time_ms": duration_ms,
            "query": query_str,
            "limit": limit,
            "offset": offset
        }
    })))
}

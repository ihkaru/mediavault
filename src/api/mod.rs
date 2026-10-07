pub mod auth;
pub mod files;
pub mod health;
pub mod resumable;
pub mod search;

use crate::state::AppState;
use axum::{
    middleware,
    routing::{delete, get, head, patch, post},
    Router,
};

pub fn build_router(state: AppState) -> Router {
    let protected_routes = Router::new()
        // File operations
        .route("/api/v1/files/upload", post(files::upload_file))
        .route("/api/v1/files", get(files::list_files))
        .route("/api/v1/files/{id}", get(files::get_file))
        .route("/api/v1/files/{id}/raw", get(files::download_raw))
        .route("/api/v1/files/{id}/thumbnail", get(files::download_thumbnail))
        .route("/api/v1/files/{id}", delete(files::delete_file))
        // Resumable upload operations
        .route("/api/v1/files/resumable", post(resumable::create_session))
        .route("/api/v1/files/resumable/{id}", head(resumable::get_session_status))
        .route("/api/v1/files/resumable/{id}", patch(resumable::append_chunk))
        // Instant search
        .route("/api/v1/search", get(search::search_files))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ));

    let public_routes = Router::new()
        .route("/healthz", get(health::health_check))
        .route("/metrics", get(health::metrics));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .with_state(state)
}

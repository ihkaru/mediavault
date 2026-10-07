use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

pub async fn require_api_key(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    if let Some(ref required_key) = state.config.api_key {
        let provided_key = req
            .headers()
            .get("x-api-key")
            .and_then(|h| h.to_str().ok())
            .or_else(|| {
                req.headers()
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|auth| auth.strip_prefix("Bearer "))
            });

        match provided_key {
            Some(key) if key == required_key => {}
            _ => {
                return Err(AppError::Unauthorized(
                    "Invalid or missing API key in X-API-Key or Authorization header".to_string(),
                ));
            }
        }
    }

    Ok(next.run(req).await)
}

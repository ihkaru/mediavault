use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use mediavault::{
    api::build_router,
    config::Config,
    db::init_db,
    search::MeiliClient,
    state::AppState,
    storage::local::LocalStorage,
};
use serde_json::Value;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempdir().expect("Failed to create tempdir");
    let db_path = dir.path().join("test.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let storage_dir = dir.path().join("storage");
    let temp_storage_dir = dir.path().join("temp");

    let pool = init_db(&db_url).await.expect("Failed to init test DB");
    let storage = Arc::new(LocalStorage::new(&storage_dir).expect("Failed to init storage"));

    let mut config = Config::from_env();
    config.storage_local_dir = storage_dir.to_string_lossy().to_string();
    config.storage_temp_dir = temp_storage_dir.to_string_lossy().to_string();
    config.meili_enabled = false; // Test database fallback search
    config.api_key = Some("test-secret-key".to_string());

    let meili = MeiliClient::new(
        "http://localhost:7700".to_string(),
        None,
        "test_files".to_string(),
        false,
    );

    let state = AppState::new(config, pool, storage, meili);
    let app = build_router(state);

    (app, dir)
}

#[tokio::test]
async fn test_health_and_metrics_endpoints() {
    let (app, _dir) = setup_test_app().await;

    // 1. Healthz
    let req = Request::builder()
        .uri("/healthz")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["service"], "mediavault");
    assert_eq!(json["status"], "healthy");

    // 2. Metrics
    let req = Request::builder()
        .uri("/metrics")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["total_files"], 0);
}

#[tokio::test]
async fn test_auth_rejection_without_api_key() {
    let (app, _dir) = setup_test_app().await;

    // Calling protected route without API key
    let req = Request::builder()
        .uri("/api/v1/files")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_resumable_upload_workflow() {
    let (app, _dir) = setup_test_app().await;

    let payload = serde_json::json!({
        "filename": "document.txt",
        "total_size": 18,
        "mime_type": "text/plain",
        "app_id": "test-app"
    });

    // 1. Create resumable session
    let req = Request::builder()
        .uri("/api/v1/files/resumable")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-api-key", "test-secret-key")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let session_id = json["data"]["id"].as_str().unwrap().to_string();

    // 2. Check session status (HEAD)
    let req = Request::builder()
        .uri(format!("/api/v1/files/resumable/{}", session_id))
        .method("HEAD")
        .header("x-api-key", "test-secret-key")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("upload-offset").unwrap(), "0");

    // 3. Patch chunk 1: 10 bytes ("HelloWorld")
    let chunk1 = b"HelloWorld";
    let req = Request::builder()
        .uri(format!("/api/v1/files/resumable/{}", session_id))
        .method("PATCH")
        .header("x-api-key", "test-secret-key")
        .header("upload-offset", "0")
        .body(Body::from(chunk1.to_vec()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(resp.headers().get("upload-offset").unwrap(), "10");

    // 4. Patch chunk 2: remaining 8 bytes ("12345678") -> Completes upload!
    let chunk2 = b"12345678";
    let req = Request::builder()
        .uri(format!("/api/v1/files/resumable/{}", session_id))
        .method("PATCH")
        .header("x-api-key", "test-secret-key")
        .header("upload-offset", "10")
        .body(Body::from(chunk2.to_vec()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let final_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(final_json["status"], "completed");
    let file_id = final_json["data"]["id"].as_str().unwrap().to_string();

    // 5. Query instant search fallback
    let req = Request::builder()
        .uri("/api/v1/search?q=document")
        .method("GET")
        .header("x-api-key", "test-secret-key")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let search_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(search_json["engine"], "database_fallback");
    assert_eq!(search_json["data"]["hits"].as_array().unwrap().len(), 1);
    assert_eq!(search_json["data"]["hits"][0]["id"], file_id);
}

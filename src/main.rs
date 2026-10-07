use mediavault::{
    config::Config,
    db,
    api,
    search::MeiliClient,
    state::AppState,
    storage::{local::LocalStorage, s3::S3Storage, DynStorage},
};
use axum::extract::DefaultBodyLimit;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env();

    // Initialize structured logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mediavault=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        storage_backend = %config.storage_backend,
        "Starting MediaVault daemon"
    );

    // Initialize database pool & migrations
    let db_pool = db::init_db(&config.database_url).await?;

    // Initialize storage backend
    let storage_backend: DynStorage = if config.storage_backend == "s3" {
        if let Some(endpoint) = config.s3_endpoint.clone() {
            tracing::info!(endpoint = %endpoint, bucket = %config.s3_bucket, "Using Garage/S3 storage backend");
            Arc::new(S3Storage::new(
                endpoint,
                config.s3_bucket.clone(),
                config.s3_access_key.clone(),
                config.s3_secret_key.clone(),
                config.s3_public_url.clone(),
            ))
        } else {
            tracing::warn!("S3 storage requested but S3_ENDPOINT not set; falling back to local storage");
            Arc::new(LocalStorage::new(&config.storage_local_dir)?)
        }
    } else {
        tracing::info!(path = %config.storage_local_dir, "Using Local filesystem storage backend");
        Arc::new(LocalStorage::new(&config.storage_local_dir)?)
    };

    // Initialize Meilisearch client
    let meili_client = MeiliClient::new(
        config.meili_host.clone(),
        config.meili_api_key.clone(),
        config.meili_index.clone(),
        config.meili_enabled,
    );

    // Ensure Meilisearch index & search attributes asynchronously
    if config.meili_enabled {
        let meili_clone = meili_client.clone();
        tokio::spawn(async move {
            if let Err(e) = meili_clone.ensure_index_and_settings().await {
                tracing::warn!("Could not connect to Meilisearch on startup: {}. Fallback search is active.", e);
            }
        });
    }

    let state = AppState::new(config.clone(), db_pool, storage_backend, meili_client);

    let max_body_bytes = config.max_upload_size_mb * 1024 * 1024;

    // Build Axum application router
    let app = api::build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(CompressionLayer::new())
        .layer(DefaultBodyLimit::max(max_body_bytes));

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST or PORT configuration");

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("MediaVault listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("MediaVault shutdown completed cleanly");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C signal handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, initiating graceful shutdown");
        },
        _ = terminate => {
            tracing::info!("Received SIGTERM, initiating graceful shutdown");
        },
    }
}

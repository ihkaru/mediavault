pub mod models;

use anyhow::Context;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::fs;
use std::path::Path;

pub type DbPool = Pool<Sqlite>;

pub async fn init_db(database_url: &str) -> anyhow::Result<DbPool> {
    if database_url.starts_with("sqlite:") {
        let clean_path = database_url
            .trim_start_matches("sqlite:")
            .split('?')
            .next()
            .unwrap_or("");

        if let Some(parent) = Path::new(clean_path).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create DB directory: {:?}", parent))?;
            }
        }
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(20)
        .connect(database_url)
        .await
        .with_context(|| format!("Failed to connect to SQLite at {}", database_url))?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS files (
            id TEXT PRIMARY KEY,
            app_id TEXT NOT NULL,
            filename TEXT NOT NULL,
            mime_type TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            storage_backend TEXT NOT NULL,
            storage_path TEXT NOT NULL,
            has_thumbnail BOOLEAN NOT NULL DEFAULT 0,
            thumbnail_path TEXT,
            extracted_text TEXT,
            tags TEXT,
            metadata_json TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_files_app_id ON files(app_id);
        CREATE INDEX IF NOT EXISTS idx_files_sha256 ON files(sha256);
        CREATE INDEX IF NOT EXISTS idx_files_mime_type ON files(mime_type);
        CREATE INDEX IF NOT EXISTS idx_files_created_at ON files(created_at);

        CREATE TABLE IF NOT EXISTS resumable_sessions (
            id TEXT PRIMARY KEY,
            app_id TEXT NOT NULL,
            filename TEXT NOT NULL,
            mime_type TEXT NOT NULL,
            total_size INTEGER NOT NULL,
            current_offset INTEGER NOT NULL DEFAULT 0,
            temp_file_path TEXT NOT NULL,
            metadata_json TEXT,
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_resumable_sessions_expires_at ON resumable_sessions(expires_at);
        "#,
    )
    .execute(&pool)
    .await
    .context("Failed to run initial SQLite database migrations")?;

    tracing::info!("Database initialized successfully with active schema");
    Ok(pool)
}

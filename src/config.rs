use std::env;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub storage_backend: String,
    pub storage_local_dir: String,
    pub storage_temp_dir: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: String,
    pub s3_region: String,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
    pub s3_public_url: Option<String>,
    pub meili_enabled: bool,
    pub meili_host: String,
    pub meili_api_key: Option<String>,
    pub meili_index: String,
    pub api_key: Option<String>,
    pub max_upload_size_mb: usize,
    pub thumbnail_max_width: u32,
    pub thumbnail_max_height: u32,
}

impl Config {
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);

        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite:./data/mediavault.db?mode=rwc".to_string());

        let storage_backend = env::var("STORAGE_BACKEND")
            .unwrap_or_else(|_| "local".to_string())
            .to_lowercase();

        let storage_local_dir = env::var("STORAGE_LOCAL_DIR")
            .unwrap_or_else(|_| "./data/storage".to_string());

        let storage_temp_dir = env::var("STORAGE_TEMP_DIR")
            .unwrap_or_else(|_| "./data/temp".to_string());

        let s3_endpoint = env::var("S3_ENDPOINT").ok();
        let s3_bucket = env::var("S3_BUCKET").unwrap_or_else(|_| "mediavault".to_string());
        let s3_region = env::var("S3_REGION").unwrap_or_else(|_| "garage".to_string());
        let s3_access_key = env::var("S3_ACCESS_KEY").ok();
        let s3_secret_key = env::var("S3_SECRET_KEY").ok();
        let s3_public_url = env::var("S3_PUBLIC_URL").ok();

        let meili_enabled = env::var("MEILI_ENABLED")
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let meili_host = env::var("MEILI_HOST")
            .unwrap_or_else(|_| "http://localhost:7700".to_string());

        let meili_api_key = env::var("MEILI_API_KEY").ok().filter(|s| !s.is_empty());
        let meili_index = env::var("MEILI_INDEX")
            .unwrap_or_else(|_| "mediavault_files".to_string());

        let api_key = env::var("API_KEY").ok().filter(|s| !s.is_empty());

        let max_upload_size_mb = env::var("MAX_UPLOAD_SIZE_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(250);

        let thumbnail_max_width = env::var("THUMBNAIL_MAX_WIDTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);

        let thumbnail_max_height = env::var("THUMBNAIL_MAX_HEIGHT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);

        Self {
            host,
            port,
            database_url,
            storage_backend,
            storage_local_dir,
            storage_temp_dir,
            s3_endpoint,
            s3_bucket,
            s3_region,
            s3_access_key,
            s3_secret_key,
            s3_public_url,
            meili_enabled,
            meili_host,
            meili_api_key,
            meili_index,
            api_key,
            max_upload_size_mb,
            thumbnail_max_width,
            thumbnail_max_height,
        }
    }
}

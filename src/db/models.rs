use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FileRecord {
    pub id: String,
    pub app_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    pub sha256: String,
    pub storage_backend: String,
    pub storage_path: String,
    pub has_thumbnail: bool,
    pub thumbnail_path: Option<String>,
    pub extracted_text: Option<String>,
    pub tags: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResumableSession {
    pub id: String,
    pub app_id: String,
    pub filename: String,
    pub mime_type: String,
    pub total_size: i64,
    pub current_offset: i64,
    pub temp_file_path: String,
    pub metadata_json: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub id: String,
    pub app_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    pub sha256: String,
    pub has_thumbnail: bool,
    pub thumbnail_url: Option<String>,
    pub download_url: String,
    pub tags: Vec<String>,
    pub snippet: Option<String>,
    pub created_at: String,
}

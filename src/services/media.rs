use crate::{
    config::Config,
    db::{
        models::{FileRecord, ResumableSession, SearchHit},
        DbPool,
    },
    error::AppError,
    processor::analyze_and_process,
    search::{meili::MeiliDocument, MeiliClient},
    storage::DynStorage,
};
use chrono::{Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub struct NewFileInput {
    pub app_id: String,
    pub filename: String,
    pub file_bytes: Vec<u8>,
    pub tags: Vec<String>,
    pub metadata_json: Option<String>,
}

pub enum ChunkResult {
    Appended { current_offset: i64 },
    Completed { record: FileRecord },
}

pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub estimated_total_hits: usize,
    pub processing_time_ms: u64,
    pub engine: &'static str,
    pub query: String,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone)]
pub struct MediaService {
    config: Arc<Config>,
    db: DbPool,
    storage: DynStorage,
    search: Arc<MeiliClient>,
}

impl MediaService {
    pub fn new(
        config: Arc<Config>,
        db: DbPool,
        storage: DynStorage,
        search: Arc<MeiliClient>,
    ) -> Self {
        Self {
            config,
            db,
            storage,
            search,
        }
    }

    pub fn storage(&self) -> &DynStorage {
        &self.storage
    }

    pub fn db(&self) -> &DbPool {
        &self.db
    }

    pub fn search_client(&self) -> &Arc<MeiliClient> {
        &self.search
    }

    /// Process and persist a new uploaded file
    pub async fn save_file(&self, input: NewFileInput) -> Result<FileRecord, AppError> {
        if input.file_bytes.is_empty() {
            return Err(AppError::BadRequest("File payload is empty".to_string()));
        }

        let file_size = input.file_bytes.len() as i64;
        let max_bytes = (self.config.max_upload_size_mb * 1024 * 1024) as i64;
        if file_size > max_bytes {
            return Err(AppError::BadRequest(format!(
                "File size {} bytes exceeds maximum limit {} MB",
                file_size, self.config.max_upload_size_mb
            )));
        }

        // Deduce MIME
        let mime_type = mime_guess::from_path(&input.filename)
            .first_or_octet_stream()
            .to_string();

        // Calculate SHA-256 fingerprint
        let mut hasher = Sha256::new();
        hasher.update(&input.file_bytes);
        let sha256 = hex::encode(hasher.finalize());

        let file_id = Uuid::new_v4().to_string();
        let storage_path = format!("raw/{}/{}", file_id, input.filename);

        // Store raw object
        self.storage
            .put_object(&storage_path, &input.file_bytes, &mime_type)
            .await
            .map_err(|e| AppError::Storage(format!("Failed saving to storage: {}", e)))?;

        // Process media
        let processed = analyze_and_process(
            &mime_type,
            &input.file_bytes,
            self.config.thumbnail_max_width,
            self.config.thumbnail_max_height,
        );

        let mut has_thumbnail = false;
        let mut thumbnail_path: Option<String> = None;

        if let Some(ref thumb_bytes) = processed.thumbnail_bytes {
            let t_path = format!("thumbnails/{}.webp", file_id);
            if self
                .storage
                .put_object(&t_path, thumb_bytes, "image/webp")
                .await
                .is_ok()
            {
                has_thumbnail = true;
                thumbnail_path = Some(t_path);
            }
        }

        let now = Utc::now().to_rfc3339();
        let tags_str = if input.tags.is_empty() {
            None
        } else {
            Some(input.tags.join(","))
        };

        let record = FileRecord {
            id: file_id.clone(),
            app_id: input.app_id.clone(),
            filename: input.filename.clone(),
            mime_type: mime_type.clone(),
            file_size,
            sha256: sha256.clone(),
            storage_backend: self.storage.name().to_string(),
            storage_path,
            has_thumbnail,
            thumbnail_path,
            extracted_text: processed.extracted_text.clone(),
            tags: tags_str,
            metadata_json: input.metadata_json.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        // Insert to DB
        sqlx::query(
            r#"
            INSERT INTO files (
                id, app_id, filename, mime_type, file_size, sha256,
                storage_backend, storage_path, has_thumbnail, thumbnail_path,
                extracted_text, tags, metadata_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
        )
        .bind(&record.id)
        .bind(&record.app_id)
        .bind(&record.filename)
        .bind(&record.mime_type)
        .bind(record.file_size)
        .bind(&record.sha256)
        .bind(&record.storage_backend)
        .bind(&record.storage_path)
        .bind(record.has_thumbnail)
        .bind(&record.thumbnail_path)
        .bind(&record.extracted_text)
        .bind(&record.tags)
        .bind(&record.metadata_json)
        .bind(&record.created_at)
        .bind(&record.updated_at)
        .execute(&self.db)
        .await
        .map_err(AppError::Database)?;

        // Asynchronously sync to search engine
        let search_client = self.search.clone();
        let meili_doc = MeiliDocument {
            id: record.id.clone(),
            app_id: record.app_id.clone(),
            filename: record.filename.clone(),
            mime_type: record.mime_type.clone(),
            file_size: record.file_size,
            sha256: record.sha256.clone(),
            has_thumbnail: record.has_thumbnail,
            tags: input.tags,
            extracted_text: record.extracted_text.clone(),
            metadata_json: record.metadata_json.clone(),
            created_at: record.created_at.clone(),
        };

        tokio::spawn(async move {
            let _ = search_client.index_document(&meili_doc).await;
        });

        Ok(record)
    }

    /// Retrieve file metadata by ID
    pub async fn get_file(&self, id: &str) -> Result<FileRecord, AppError> {
        let record: Option<FileRecord> = sqlx::query_as("SELECT * FROM files WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.db)
            .await
            .map_err(AppError::Database)?;

        record.ok_or_else(|| AppError::NotFound(format!("File {} not found", id)))
    }

    /// Read raw bytes from storage
    pub async fn get_raw_bytes(&self, path: &str) -> Result<Vec<u8>, AppError> {
        self.storage
            .get_object(path)
            .await
            .map_err(|e| AppError::Storage(format!("Failed reading object: {}", e)))
    }

    /// Delete file across storage, DB, and search index
    pub async fn delete_file(&self, id: &str) -> Result<(), AppError> {
        let record = self.get_file(id).await?;

        let _ = self.storage.delete_object(&record.storage_path).await;
        if let Some(ref t_path) = record.thumbnail_path {
            let _ = self.storage.delete_object(t_path).await;
        }

        sqlx::query("DELETE FROM files WHERE id = ?1")
            .bind(id)
            .execute(&self.db)
            .await
            .map_err(AppError::Database)?;

        let search_client = self.search.clone();
        let doc_id = id.to_string();
        tokio::spawn(async move {
            let _ = search_client.delete_document(&doc_id).await;
        });

        Ok(())
    }

    /// List files with optional filtering
    pub async fn list_files(
        &self,
        app_id: Option<&str>,
        mime_type: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<FileRecord>, AppError> {
        let limit = limit.clamp(1, 100);
        let offset = offset.max(0);

        let mut query_builder = sqlx::QueryBuilder::new("SELECT * FROM files WHERE 1=1 ");

        if let Some(app) = app_id {
            query_builder.push(" AND app_id = ");
            query_builder.push_bind(app);
        }

        if let Some(mime) = mime_type {
            if mime.ends_with('*') {
                let prefix = format!("{}%", mime.trim_end_matches('*'));
                query_builder.push(" AND mime_type LIKE ");
                query_builder.push_bind(prefix);
            } else {
                query_builder.push(" AND mime_type = ");
                query_builder.push_bind(mime);
            }
        }

        query_builder.push(" ORDER BY created_at DESC LIMIT ");
        query_builder.push_bind(limit);
        query_builder.push(" OFFSET ");
        query_builder.push_bind(offset);

        let records: Vec<FileRecord> = query_builder
            .build_query_as()
            .fetch_all(&self.db)
            .await
            .map_err(AppError::Database)?;

        Ok(records)
    }

    /// Initialize a resumable upload session
    pub async fn create_resumable_session(
        &self,
        filename: String,
        total_size: i64,
        mime_type: String,
        app_id: String,
        tags: Option<Vec<String>>,
    ) -> Result<ResumableSession, AppError> {
        let max_bytes = (self.config.max_upload_size_mb * 1024 * 1024) as i64;
        if total_size > max_bytes {
            return Err(AppError::BadRequest(format!(
                "Total file size {} exceeds limit {} MB",
                total_size, self.config.max_upload_size_mb
            )));
        }

        let session_id = Uuid::new_v4().to_string();
        let temp_dir = PathBuf::from(&self.config.storage_temp_dir);
        let _ = fs::create_dir_all(&temp_dir).await;
        let temp_file_path = temp_dir.join(format!("{}.part", session_id));

        let _ = fs::File::create(&temp_file_path).await.map_err(|e| {
            AppError::Storage(format!("Failed to create temporary session file: {}", e))
        })?;

        let now = Utc::now();
        let expires_at = (now + Duration::hours(24)).to_rfc3339();
        let created_at = now.to_rfc3339();
        let metadata_json = tags.map(|t| json!(t).to_string());

        let session = ResumableSession {
            id: session_id.clone(),
            app_id,
            filename,
            mime_type,
            total_size,
            current_offset: 0,
            temp_file_path: temp_file_path.to_string_lossy().to_string(),
            metadata_json,
            created_at,
            expires_at: expires_at.clone(),
        };

        sqlx::query(
            r#"
            INSERT INTO resumable_sessions (
                id, app_id, filename, mime_type, total_size,
                current_offset, temp_file_path, metadata_json, created_at, expires_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(&session.id)
        .bind(&session.app_id)
        .bind(&session.filename)
        .bind(&session.mime_type)
        .bind(session.total_size)
        .bind(session.current_offset)
        .bind(&session.temp_file_path)
        .bind(&session.metadata_json)
        .bind(&session.created_at)
        .bind(&session.expires_at)
        .execute(&self.db)
        .await
        .map_err(AppError::Database)?;

        Ok(session)
    }

    /// Retrieve resumable upload session
    pub async fn get_resumable_session(&self, id: &str) -> Result<ResumableSession, AppError> {
        let session: Option<ResumableSession> =
            sqlx::query_as("SELECT * FROM resumable_sessions WHERE id = ?1")
                .bind(id)
                .fetch_optional(&self.db)
                .await
                .map_err(AppError::Database)?;

        session.ok_or_else(|| AppError::NotFound(format!("Session {} not found", id)))
    }

    /// Append chunk bytes to active session
    pub async fn append_resumable_chunk(
        &self,
        session_id: &str,
        chunk_bytes: &[u8],
        expected_offset: Option<i64>,
    ) -> Result<ChunkResult, AppError> {
        let mut session = self.get_resumable_session(session_id).await?;

        if let Some(offset) = expected_offset {
            if offset != session.current_offset {
                return Err(AppError::BadRequest(format!(
                    "Offset mismatch: expected {}, received {}",
                    session.current_offset, offset
                )));
            }
        }

        let chunk_len = chunk_bytes.len() as i64;
        let new_offset = session.current_offset + chunk_len;

        if new_offset > session.total_size {
            return Err(AppError::BadRequest(format!(
                "Chunk exceeds declared total size {}",
                session.total_size
            )));
        }

        let mut file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&session.temp_file_path)
            .await
            .map_err(|e| AppError::Storage(format!("Failed opening part file: {}", e)))?;

        file.write_all(chunk_bytes)
            .await
            .map_err(|e| AppError::Storage(format!("Failed appending chunk: {}", e)))?;

        session.current_offset = new_offset;

        // Check if finished
        if session.current_offset == session.total_size {
            let full_bytes = fs::read(&session.temp_file_path)
                .await
                .map_err(|e| AppError::Storage(format!("Failed reading completed part: {}", e)))?;

            let tags: Vec<String> = session
                .metadata_json
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();

            let input = NewFileInput {
                app_id: session.app_id,
                filename: session.filename,
                file_bytes: full_bytes,
                tags,
                metadata_json: session.metadata_json,
            };

            let record = self.save_file(input).await?;

            // Cleanup temp file & session
            let _ = fs::remove_file(&session.temp_file_path).await;
            let _ = sqlx::query("DELETE FROM resumable_sessions WHERE id = ?1")
                .bind(session_id)
                .execute(&self.db)
                .await;

            return Ok(ChunkResult::Completed { record });
        }

        sqlx::query("UPDATE resumable_sessions SET current_offset = ?1 WHERE id = ?2")
            .bind(session.current_offset)
            .bind(session_id)
            .execute(&self.db)
            .await
            .map_err(AppError::Database)?;

        Ok(ChunkResult::Appended {
            current_offset: session.current_offset,
        })
    }

    /// Fast search with Meilisearch and seamless SQLite fallback
    pub async fn search_files(
        &self,
        query: &str,
        app_id: Option<&str>,
        mime_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<SearchResult, AppError> {
        let query_str = query.trim().to_string();
        let limit = limit.clamp(1, 100);

        // 1. Meilisearch
        if self.search.is_enabled() {
            if let Ok(resp) = self
                .search
                .search(&query_str, app_id, mime_type, limit, offset)
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

                return Ok(SearchResult {
                    hits,
                    estimated_total_hits: resp.estimated_total_hits.unwrap_or(0),
                    processing_time_ms: resp.processing_time_ms.unwrap_or(0),
                    engine: "meilisearch",
                    query: query_str,
                    limit,
                    offset,
                });
            }
        }

        // 2. Database Fallback
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

        if let Some(app) = app_id {
            builder.push(" AND app_id = ");
            builder.push_bind(app);
        }

        if let Some(mime) = mime_type {
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
            .fetch_all(&self.db)
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

        let elapsed = start_time.elapsed().as_millis() as u64;

        Ok(SearchResult {
            hits,
            estimated_total_hits: 0,
            processing_time_ms: elapsed,
            engine: "database_fallback",
            query: query_str,
            limit,
            offset,
        })
    }
}

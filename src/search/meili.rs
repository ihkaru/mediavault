use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug)]
pub struct MeiliClient {
    client: Client,
    host: String,
    api_key: Option<String>,
    index: String,
    enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeiliDocument {
    pub id: String,
    pub app_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    pub sha256: String,
    pub has_thumbnail: bool,
    pub tags: Vec<String>,
    pub extracted_text: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct MeiliSearchResponse {
    pub hits: Vec<MeiliDocument>,
    #[serde(rename = "estimatedTotalHits")]
    pub estimated_total_hits: Option<usize>,
    #[serde(rename = "processingTimeMs")]
    pub processing_time_ms: Option<u64>,
}

impl MeiliClient {
    pub fn new(host: String, api_key: Option<String>, index: String, enabled: bool) -> Self {
        let clean_host = host.trim_end_matches('/').to_string();
        Self {
            client: Client::new(),
            host: clean_host,
            api_key,
            index,
            enabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn auth_request(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            req.bearer_auth(key)
        } else {
            req
        }
    }

    pub async fn ensure_index_and_settings(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // 1. Create index if not exists
        let create_url = format!("{}/indexes", self.host);
        let req = self
            .auth_request(self.client.post(&create_url))
            .json(&json!({
                "uid": self.index,
                "primaryKey": "id"
            }));

        let _ = req.send().await;

        // 2. Configure filterable & searchable attributes
        let settings_url = format!("{}/indexes/{}/settings", self.host, self.index);
        let settings_body = json!({
            "filterableAttributes": ["app_id", "mime_type", "tags", "created_at"],
            "searchableAttributes": ["filename", "extracted_text", "tags", "metadata_json"],
            "sortableAttributes": ["created_at", "file_size"]
        });

        let req = self
            .auth_request(self.client.patch(&settings_url))
            .json(&settings_body);

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to configure Meilisearch index settings at {}", settings_url))?;

        if resp.status().is_success() {
            tracing::info!(
                index = %self.index,
                "Meilisearch index & filterable settings confirmed"
            );
        }

        Ok(())
    }

    pub async fn index_document(&self, doc: &MeiliDocument) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let url = format!("{}/indexes/{}/documents", self.host, self.index);
        let req = self
            .auth_request(self.client.post(&url))
            .json(&vec![doc]);

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to send document to Meilisearch index {}", self.index))?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            tracing::warn!("Failed indexing document to Meilisearch: {}", err);
        }

        Ok(())
    }

    pub async fn delete_document(&self, doc_id: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let url = format!("{}/indexes/{}/documents/{}", self.host, self.index, doc_id);
        let req = self.auth_request(self.client.delete(&url));
        let _ = req.send().await;
        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        filter_app_id: Option<&str>,
        filter_mime_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<MeiliSearchResponse> {
        let url = format!("{}/indexes/{}/search", self.host, self.index);

        let mut filters = Vec::new();
        if let Some(app) = filter_app_id {
            filters.push(format!("app_id = \"{}\"", app));
        }
        if let Some(mime) = filter_mime_type {
            if mime.ends_with('*') {
                let prefix = mime.trim_end_matches('*');
                filters.push(format!("mime_type STARTS_WITH \"{}\"", prefix));
            } else {
                filters.push(format!("mime_type = \"{}\"", mime));
            }
        }

        let filter_str = if filters.is_empty() {
            None
        } else {
            Some(filters.join(" AND "))
        };

        let body = json!({
            "q": query,
            "filter": filter_str,
            "limit": limit,
            "offset": offset,
            "attributesToHighlight": ["filename", "extracted_text"]
        });

        let req = self.auth_request(self.client.post(&url)).json(&body);
        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed searching Meilisearch index {}", self.index))?;

        let search_resp = resp
            .json::<MeiliSearchResponse>()
            .await
            .context("Failed parsing Meilisearch JSON response")?;

        Ok(search_resp)
    }

    pub async fn is_healthy(&self) -> bool {
        if !self.enabled {
            return true;
        }

        let url = format!("{}/health", self.host);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }
}

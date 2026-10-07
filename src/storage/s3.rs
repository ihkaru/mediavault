use super::StorageBackend;
use anyhow::{bail, Context, Result};
use reqwest::Client;

#[allow(dead_code)]
pub struct S3Storage {
    client: Client,
    endpoint: String,
    bucket: String,
    public_url: Option<String>,
}

#[allow(dead_code)]
impl S3Storage {
    pub fn new(
        endpoint: String,
        bucket: String,
        _access_key: Option<String>,
        _secret_key: Option<String>,
        public_url: Option<String>,
    ) -> Self {
        let clean_endpoint = endpoint.trim_end_matches('/').to_string();
        Self {
            client: Client::new(),
            endpoint: clean_endpoint,
            bucket,
            public_url,
        }
    }

    fn object_url(&self, path: &str) -> String {
        let clean_path = path.trim_start_matches('/');
        format!("{}/{}/{}", self.endpoint, self.bucket, clean_path)
    }

    pub fn public_url(&self, path: &str) -> Option<String> {
        let clean_path = path.trim_start_matches('/');
        if let Some(ref base) = self.public_url {
            Some(format!("{}/{}/{}", base.trim_end_matches('/'), self.bucket, clean_path))
        } else {
            None
        }
    }
}

impl StorageBackend for S3Storage {
    fn name(&self) -> &'static str {
        "s3"
    }

    fn put_object<'a>(
        &'a self,
        path: &'a str,
        data: &'a [u8],
        content_type: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let url = self.object_url(path);
            let response = self
                .client
                .put(&url)
                .header("Content-Type", content_type)
                .body(data.to_vec())
                .send()
                .await
                .with_context(|| format!("Failed to send PUT request to S3 at {}", url))?;

            if !response.status().is_success() {
                let status = response.status();
                let err_text = response.text().await.unwrap_or_default();
                bail!("S3 PUT failed with status {}: {}", status, err_text);
            }

            Ok(())
        })
    }

    fn get_object<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move {
            let url = self.object_url(path);
            let response = self
                .client
                .get(&url)
                .send()
                .await
                .with_context(|| format!("Failed to send GET request to S3 at {}", url))?;

            if !response.status().is_success() {
                let status = response.status();
                bail!("S3 GET failed with status {}", status);
            }

            let bytes = response
                .bytes()
                .await
                .with_context(|| format!("Failed to read S3 bytes from {}", url))?;

            Ok(bytes.to_vec())
        })
    }

    fn delete_object<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let url = self.object_url(path);
            let response = self
                .client
                .delete(&url)
                .send()
                .await
                .with_context(|| format!("Failed to send DELETE request to S3 at {}", url))?;

            if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
                let status = response.status();
                bail!("S3 DELETE failed with status {}", status);
            }

            Ok(())
        })
    }

    fn exists<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async move {
            let url = self.object_url(path);
            let response = self
                .client
                .head(&url)
                .send()
                .await
                .with_context(|| format!("Failed to send HEAD request to S3 at {}", url))?;

            Ok(response.status().is_success())
        })
    }
}

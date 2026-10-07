use super::StorageBackend;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;

pub struct LocalStorage {
    base_dir: PathBuf,
}

impl LocalStorage {
    pub fn new(base_dir: impl AsRef<Path>) -> Result<Self> {
        let path = base_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create local storage dir: {:?}", path))?;
        Ok(Self { base_dir: path })
    }

    fn resolve_path(&self, relative: &str) -> PathBuf {
        let clean = relative.trim_start_matches('/');
        self.base_dir.join(clean)
    }
}

impl StorageBackend for LocalStorage {
    fn name(&self) -> &'static str {
        "local"
    }

    fn put_object<'a>(
        &'a self,
        path: &'a str,
        data: &'a [u8],
        _content_type: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let full_path = self.resolve_path(path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("Failed to create directories for: {:?}", parent))?;
            }
            fs::write(&full_path, data)
                .await
                .with_context(|| format!("Failed to write object to: {:?}", full_path))?;
            Ok(())
        })
    }

    fn get_object<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move {
            let full_path = self.resolve_path(path);
            let bytes = fs::read(&full_path)
                .await
                .with_context(|| format!("Failed to read object from: {:?}", full_path))?;
            Ok(bytes)
        })
    }

    fn delete_object<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let full_path = self.resolve_path(path);
            if full_path.exists() {
                fs::remove_file(&full_path)
                    .await
                    .with_context(|| format!("Failed to remove object at: {:?}", full_path))?;
            }
            Ok(())
        })
    }

    fn exists<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async move {
            let full_path = self.resolve_path(path);
            Ok(full_path.exists())
        })
    }
}

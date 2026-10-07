pub mod local;
pub mod s3;

use anyhow::Result;
use std::sync::Arc;

#[allow(dead_code)]
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn put_object<'a>(
        &'a self,
        path: &'a str,
        data: &'a [u8],
        content_type: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;

    fn get_object<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send + 'a>>;

    fn delete_object<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>;

    fn exists<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>>;
}

pub type DynStorage = Arc<dyn StorageBackend>;

//! Distributed file storage abstraction layer.
//!
//! Provides a unified [`Storage`] trait for local filesystem and S3-compatible object storage.
//! The backend is selected at runtime via the `STORAGE_DRIVER` environment variable with zero code changes.

pub mod local;

#[cfg(feature = "storage-s3")]
pub mod s3;

use std::time::Duration;

use crate::errors::app_error::AppResult;
use async_trait::async_trait;

/// Unified file storage interface.
///
/// All backends (LocalFS, S3, etc.) implement this trait.
/// The business layer calls via `Arc<dyn Storage>` without worrying about the underlying implementation.
#[async_trait]
pub trait Storage: Send + Sync + std::fmt::Debug {
    /// Store a file.
    ///
    /// `key` is a relative path, e.g. `blog/2026/04/a1b2c3d4.jpg`.
    async fn put(&self, key: &str, data: &[u8], content_type: &str) -> AppResult<()>;

    /// Read file contents.
    async fn get(&self, key: &str) -> AppResult<Vec<u8>>;

    /// Delete a file.
    async fn delete(&self, key: &str) -> AppResult<()>;

    /// Get the public access URL for a file.
    async fn url(&self, key: &str) -> AppResult<String>;

    /// Generate a presigned upload URL (available in S3 mode; LocalFS returns an empty string).
    async fn presigned_upload(&self, _key: &str, _ttl: Duration) -> AppResult<String> {
        Ok(String::new())
    }
}

/// Create the appropriate storage instance based on AppConfig.
pub fn create_storage(
    config: &crate::config::app::AppConfig,
) -> AppResult<std::sync::Arc<dyn Storage>> {
    match config.storage_driver.as_str() {
        "local" => {
            tracing::info!(upload_dir = %config.upload_dir, "Storage driver: local");
            Ok(std::sync::Arc::new(local::LocalStorage::new(
                &config.upload_dir,
                &config.base_url,
            )?))
        }
        #[cfg(feature = "storage-s3")]
        "s3" => {
            tracing::info!(bucket = %config.s3_bucket, "Storage driver: s3");
            Ok(std::sync::Arc::new(s3::S3Storage::from_config(config)?))
        }
        #[cfg(not(feature = "storage-s3"))]
        "s3" => {
            tracing::error!("storage-s3 feature not enabled");
            Err(crate::errors::app_error::AppError::BadRequest(
                "storage-s3 feature not enabled".into(),
            ))
        }
        other => Err(crate::errors::app_error::AppError::BadRequest(format!(
            "unknown STORAGE_DRIVER: {other}"
        ))),
    }
}

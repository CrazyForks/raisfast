//! 分布式文件存储抽象层。
//!
//! 通过 [`Storage`] trait 统一本地文件系统和 S3 兼容对象存储的访问接口，
//! 运行时通过 `STORAGE_DRIVER` 环境变量切换后端，代码零改动。

pub mod local;

#[cfg(feature = "storage-s3")]
pub mod s3;

use std::time::Duration;

use crate::errors::app_error::AppResult;
use async_trait::async_trait;

/// 文件存储统一接口。
///
/// 所有后端（LocalFS、S3 等）均实现此 trait。
/// 业务层通过 `Arc<dyn Storage>` 调用，无需关心底层实现。
#[async_trait]
pub trait Storage: Send + Sync + std::fmt::Debug {
    /// 存储文件。
    ///
    /// `key` 为相对路径，如 `blog/2026/04/a1b2c3d4.jpg`。
    async fn put(&self, key: &str, data: &[u8], content_type: &str) -> AppResult<()>;

    /// 读取文件内容。
    async fn get(&self, key: &str) -> AppResult<Vec<u8>>;

    /// 删除文件。
    async fn delete(&self, key: &str) -> AppResult<()>;

    /// 获取文件的公开访问 URL。
    async fn url(&self, key: &str) -> AppResult<String>;

    /// 生成预签名上传 URL（S3 模式可用，LocalFS 返回空字符串）。
    async fn presigned_upload(&self, _key: &str, _ttl: Duration) -> AppResult<String> {
        Ok(String::new())
    }
}

/// 根据 AppConfig 创建对应的存储实例。
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

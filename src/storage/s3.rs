//! S3-compatible object storage backend.
//!
//! Supports RustFS, MinIO, AWS S3, Cloudflare R2, and other S3-compatible storage.

use async_trait::async_trait;
use std::time::Duration;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::primitives::ByteStream;

use crate::errors::app_error::{AppError, AppResult};
use crate::storage::Storage;

/// S3-compatible object storage.
#[derive(Debug)]
pub struct S3Storage {
    client: S3Client,
    bucket: String,
    public_url: Option<String>,
}

impl S3Storage {
    /// Create an S3 client from AppConfig.
    pub fn from_config(config: &crate::config::app::AppConfig) -> AppResult<Self> {
        let endpoint = config
            .s3_endpoint
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("S3_ENDPOINT not set".into()))?;
        let access_key = config
            .s3_access_key
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("S3_ACCESS_KEY not set".into()))?;
        let secret_key = config
            .s3_secret_key
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("S3_SECRET_KEY not set".into()))?;
        let bucket = config.s3_bucket.clone();
        let region = config.s3_region.clone();
        let public_url = config.s3_public_url.clone();

        let config = aws_config::from_env()
            .region(aws_config::Region::new(region))
            .endpoint_url(endpoint)
            .credentials_provider(aws_sdk_s3::config::Credentials::new(
                access_key, secret_key, None, None, "env",
            ))
            .load_blocking();

        let client = S3Client::new(&config);

        tracing::info!(bucket = %bucket, "S3Storage initialized");

        Ok(Self {
            client,
            bucket,
            public_url,
        })
    }
}

#[async_trait]
impl Storage for S3Storage {
    async fn put(&self, key: &str, data: &[u8], content_type: &str) -> AppResult<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(data.to_vec()))
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("S3 put_object failed: {e}")))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<Vec<u8>> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("S3 get_object failed: {e}")))?;
        let body = resp
            .body
            .collect()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("S3 read body failed: {e}")))?;
        Ok(body.into_bytes().to_vec())
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("S3 delete_object failed: {e}")))?;
        Ok(())
    }

    async fn url(&self, key: &str) -> AppResult<String> {
        if let Some(ref public_url) = self.public_url {
            return Ok(format!("{}/{key}", public_url.trim_end_matches('/')));
        }
        Ok(format!("https://{}.s3.amazonaws.com/{key}", self.bucket))
    }

    async fn presigned_upload(&self, key: &str, ttl: Duration) -> AppResult<String> {
        use aws_sdk_s3::presigning::PresigningConfig;

        let config = PresigningConfig::expires_at(std::time::SystemTime::now() + ttl)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("presign config failed: {e}")))?;

        let presigned = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(config)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("presign failed: {e}")))?;

        Ok(presigned.uri().to_string())
    }
}

//! 缩略图生成 Handler
//!
//! 使用 `image` crate 将上传的图片缩放到指定尺寸，
//! 保存为 `{upload_dir}/thumbs/{media_id}_{size}.webp`。

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// 缩略图生成处理器
pub struct GenerateThumbnailHandler {
    pool: Pool,
    config: Arc<AppConfig>,
}

impl GenerateThumbnailHandler {
    /// 创建新的缩略图生成处理器
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }

    fn thumb_path(&self, media_id: &str, size: u32) -> PathBuf {
        PathBuf::from(&self.config.upload_dir)
            .join("thumbs")
            .join(format!("{media_id}_{size}.webp"))
    }
}

#[async_trait::async_trait]
impl JobHandler for GenerateThumbnailHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::GenerateThumbnail { media_id, size } = job else {
            return Ok(());
        };

        let media = crate::models::media::find_by_id(&self.pool, media_id, None)
            .await?
            .ok_or_else(|| crate::errors::app_error::AppError::not_found("media"))?;

        if !media.mimetype.starts_with("image/") {
            tracing::warn!(
                "[thumbnail] skipping non-image media: {} ({})",
                media_id,
                media.mimetype
            );
            return Ok(());
        }

        let src_path = PathBuf::from(&self.config.upload_dir).join(&media.filepath);
        let thumb_path = self.thumb_path(media_id, *size);

        let src = src_path.clone();
        let dst = thumb_path.clone();
        let target_size = *size;

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let img = image::open(&src).map_err(|e| {
                crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                    "open image {src:?}: {e}"
                ))
            })?;

            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                        "create dir {parent:?}: {e}"
                    ))
                })?;
            }

            let thumb = img.thumbnail(target_size, target_size);
            thumb.save(&dst).map_err(|e| {
                crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                    "save thumbnail {dst:?}: {e}"
                ))
            })?;

            Ok(())
        })
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!("spawn_blocking: {e}"))
        })??;

        tracing::info!(
            "[thumbnail] generated {}x{} thumbnail for media={}",
            size,
            size,
            media_id,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let config = Arc::new(crate::config::app::AppConfig::test_defaults());
        let handler = GenerateThumbnailHandler::new(pool, config);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}

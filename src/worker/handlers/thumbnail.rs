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

        let media = crate::models::media::find_by_id(
            &self.pool,
            media_id,
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
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
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let config = Arc::new(crate::config::app::AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            env: "test".into(),
            database_url: "sqlite::memory:".into(),
            db_pool_size: 1,
            jwt_secret: "test-secret-key-at-least-32-characters-long".into(),
            jwt_access_expires: 900,
            jwt_refresh_expires: 604800,
            upload_dir: "/tmp/test-uploads".into(),
            max_upload_size: 5242880,
            static_dir: "./static".into(),
            base_url: "http://localhost:9000".into(),
            cors_origins: None,
            tls_cert_path: None,
            tls_key_path: None,
            plugin_dir: None,
            plugin_hot_reload: false,
            plugin_max_memory_mb: 32,
            plugin_default_timeout_ms: 5000,
            plugin_disabled: vec![],
            plugin_vfs_root: "./plugins-data".into(),
            plugin_vfs_max_file_size: 1048576,
            plugin_vfs_max_total_size: 10485760,
            log_dir: "./logs".into(),
            log_max_files: 7,
            rate_limit_global_max: 60,
            rate_limit_global_window: 60,
            rate_limit_register_max: 5,
            rate_limit_register_window: 3600,
            rate_limit_login_max: 10,
            rate_limit_login_window: 60,
            rate_limit_comment_max: 3,
            rate_limit_comment_window: 60,
            worker_enabled: false,
            worker_concurrency: 1,
            worker_poll_interval_ms: 500,
            worker_default_max_attempts: 3,
            worker_cron_tick_ms: 60000,
            cron_seed_enabled: false,
            cron_schedules: vec![],
            cron_log_retention_days: 30,
            search_engine: "none".into(),
            search_index_dir: "./data/search_index".into(),
            content_type_dir: "./content_types".into(),
            timezone: "UTC".into(),
        });
        let handler = GenerateThumbnailHandler::new(pool, config);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}

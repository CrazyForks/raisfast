//! 缓存失效 Handler
//!
//! 当前仅记录需要失效的缓存键。
//! 生产环境需接入缓存后端（如 `moka`、Redis）后替换实现。

use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// 缓存失效处理器
pub struct InvalidateCacheHandler;

impl Default for InvalidateCacheHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl InvalidateCacheHandler {
    /// 创建新的缓存失效处理器
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl JobHandler for InvalidateCacheHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::InvalidateCache { keys } = job else {
            return Ok(());
        };

        tracing::info!("[cache] invalidating {} key(s): {:?}", keys.len(), keys);

        // TODO: 生产环境替换为真实缓存失效
        // for key in keys {
        //     cache.delete(key).await?;
        // }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logs_cache_invalidation() {
        let handler = InvalidateCacheHandler::new();
        let job = Job::InvalidateCache {
            keys: vec!["post:slug-hello".into(), "rss:feed".into()],
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let handler = InvalidateCacheHandler::new();
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}

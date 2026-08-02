//! Cache invalidation Handler
//!
//! Receives `InvalidateCache` jobs and deletes cache entries by key or prefix.

use std::sync::Arc;

use crate::cache::CacheStore;
use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// Cache invalidation handler
pub struct InvalidateCacheHandler {
    cache: Arc<dyn CacheStore>,
}

impl InvalidateCacheHandler {
    /// Creates a new cache invalidation handler
    #[must_use]
    pub fn new(cache: Arc<dyn CacheStore>) -> Self {
        Self { cache }
    }
}

#[async_trait::async_trait]
impl JobHandler for InvalidateCacheHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::InvalidateCache { keys } = job else {
            return Ok(());
        };

        for key in keys {
            if key.ends_with('*') {
                let prefix = &key[..key.len() - 1];
                let n = self.cache.delete_prefix(prefix).await?;
                tracing::info!("[cache] invalidated {n} key(s) with prefix '{prefix}'");
            } else {
                self.cache.delete(key).await?;
                tracing::info!("[cache] invalidated key '{key}'");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MemoryCache;
    use std::time::Duration;

    #[tokio::test]
    async fn invalidates_cache_keys() {
        let cache = Arc::new(MemoryCache::new());
        cache
            .set("post:1", "data", Some(Duration::from_secs(60)))
            .await
            .unwrap();
        cache
            .set("post:2", "data", Some(Duration::from_secs(60)))
            .await
            .unwrap();

        let handler = InvalidateCacheHandler::new(cache.clone());
        let job = Job::InvalidateCache {
            keys: vec!["post:1".into()],
        };
        handler.handle(&job).await.unwrap();

        assert!(cache.get("post:1").await.is_none());
        assert!(cache.get("post:2").await.is_some());
    }

    #[tokio::test]
    async fn invalidates_by_prefix() {
        let cache = Arc::new(MemoryCache::new());
        cache
            .set("post:1", "data", Some(Duration::from_secs(60)))
            .await
            .unwrap();
        cache
            .set("post:2", "data", Some(Duration::from_secs(60)))
            .await
            .unwrap();

        let handler = InvalidateCacheHandler::new(cache.clone());
        let job = Job::InvalidateCache {
            keys: vec!["post:*".into()],
        };
        handler.handle(&job).await.unwrap();

        assert!(cache.get("post:1").await.is_none());
        assert!(cache.get("post:2").await.is_none());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let cache = Arc::new(MemoryCache::new());
        let handler = InvalidateCacheHandler::new(cache);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}

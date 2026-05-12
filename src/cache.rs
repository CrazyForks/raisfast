//! Cache abstraction layer.
//!
//! Provides [`CacheStore`] trait and a moka-based lock-free concurrent implementation [`MemoryCache`].
//! Can be replaced with a Redis implementation in production.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::errors::app_error::AppResult;

/// Cache store interface
///
/// All cache backends (in-memory, Redis, etc.) implement this trait.
#[async_trait::async_trait]
pub trait CacheStore: Send + Sync {
    /// Get a cached value
    async fn get(&self, key: &str) -> Option<String>;

    /// Set a cached value with optional TTL
    async fn set(&self, key: &str, value: &str, ttl: Option<Duration>) -> AppResult<()>;

    /// Delete a cached value
    async fn delete(&self, key: &str) -> AppResult<()>;

    /// Bulk delete by prefix
    async fn delete_prefix(&self, prefix: &str) -> AppResult<u64>;
}

#[derive(Clone)]
struct CacheEntry {
    value: String,
    deadline: Option<std::time::Instant>,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        self.deadline
            .is_some_and(|dl| std::time::Instant::now() > dl)
    }
}

/// moka-based lock-free concurrent cache implementation
///
/// Uses TinyLFU + LRU eviction policy, no lock contention under high concurrency.
#[derive(Clone)]
pub struct MemoryCache {
    inner: moka::sync::Cache<String, CacheEntry>,
}

impl MemoryCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: moka::sync::Cache::builder().max_capacity(10_000).build(),
        }
    }
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CacheStore for MemoryCache {
    async fn get(&self, key: &str) -> Option<String> {
        let entry = self.inner.get(key)?;
        if entry.is_expired() {
            self.inner.invalidate(key);
            return None;
        }
        Some(entry.value.clone())
    }

    async fn set(&self, key: &str, value: &str, ttl: Option<Duration>) -> AppResult<()> {
        let deadline = ttl.map(|d| std::time::Instant::now() + d);
        self.inner.insert(
            key.to_string(),
            CacheEntry {
                value: value.to_string(),
                deadline,
            },
        );
        Ok(())
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.inner.invalidate(key);
        Ok(())
    }

    async fn delete_prefix(&self, prefix: &str) -> AppResult<u64> {
        let keys: Vec<Arc<String>> = self
            .inner
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k)
            .collect();
        let count = keys.len() as u64;
        for key in keys {
            self.inner.invalidate(&*key);
        }
        Ok(count)
    }
}

/// Cache helper function
///
/// Attempts to get from cache; on miss, executes `f` and backfills the cache.
pub async fn get_or<F, Fut>(
    cache: &Arc<dyn CacheStore>,
    key: &str,
    ttl: Duration,
    f: F,
) -> AppResult<String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = AppResult<String>>,
{
    if let Some(cached) = cache.get(key).await {
        return Ok(cached);
    }

    let value = f().await?;
    let _ = cache.set(key, &value, Some(ttl)).await;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_and_get() {
        let cache = MemoryCache::new();
        cache.set("k", "v", None).await.unwrap();
        assert_eq!(cache.get("k").await.unwrap(), "v");
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let cache = MemoryCache::new();
        assert!(cache.get("missing").await.is_none());
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let cache = MemoryCache::new();
        cache.set("k", "v", None).await.unwrap();
        cache.delete("k").await.unwrap();
        assert!(cache.get("k").await.is_none());
    }

    #[tokio::test]
    async fn set_overwrites() {
        let cache = MemoryCache::new();
        cache.set("k", "v1", None).await.unwrap();
        cache.set("k", "v2", None).await.unwrap();
        assert_eq!(cache.get("k").await.unwrap(), "v2");
    }

    #[tokio::test]
    async fn delete_prefix_removes_matching() {
        let cache = MemoryCache::new();
        cache.set("posts:1", "a", None).await.unwrap();
        cache.set("posts:2", "b", None).await.unwrap();
        cache.set("tags:1", "c", None).await.unwrap();
        let count = cache.delete_prefix("posts:").await.unwrap();
        assert_eq!(count, 2);
        assert!(cache.get("posts:1").await.is_none());
        assert!(cache.get("posts:2").await.is_none());
        assert_eq!(cache.get("tags:1").await.unwrap(), "c");
    }

    #[tokio::test]
    async fn expiry_with_ttl() {
        let cache = MemoryCache::new();
        cache
            .set("k", "v", Some(Duration::from_millis(1)))
            .await
            .unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(cache.get("k").await.is_none());
    }

    #[tokio::test]
    async fn get_or_cache_miss() {
        let cache: Arc<dyn CacheStore> = Arc::new(MemoryCache::new());
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count_clone = call_count.clone();
        let result = get_or(&cache, "key", Duration::from_secs(60), || {
            let c = count_clone.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok("computed".to_string())
            }
        })
        .await
        .unwrap();
        assert_eq!(result, "computed");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn get_or_cache_hit() {
        let cache: Arc<dyn CacheStore> = Arc::new(MemoryCache::new());
        cache.set("key", "cached", None).await.unwrap();
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count_clone = call_count.clone();
        let result = get_or(&cache, "key", Duration::from_secs(60), || {
            let c = count_clone.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok("computed".to_string())
            }
        })
        .await
        .unwrap();
        assert_eq!(result, "cached");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

//! 缓存抽象层。
//!
//! 提供 [`CacheStore`] trait 和基于 moka 的无锁并发实现 [`MemoryCache`]。
//! 生产环境可替换为 Redis 实现。

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::errors::app_error::AppResult;

/// 缓存存储接口
///
/// 所有缓存后端（内存、Redis 等）实现此 trait。
#[async_trait::async_trait]
pub trait CacheStore: Send + Sync {
    /// 获取缓存值
    async fn get(&self, key: &str) -> Option<String>;

    /// 设置缓存值，可选 TTL
    async fn set(&self, key: &str, value: &str, ttl: Option<Duration>) -> AppResult<()>;

    /// 删除缓存值
    async fn delete(&self, key: &str) -> AppResult<()>;

    /// 按前缀批量删除
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

/// 基于 moka 的无锁并发缓存实现
///
/// 使用 TinyLFU + LRU 淘汰策略，高并发下无锁竞争。
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

/// 缓存辅助函数
///
/// 尝试从缓存获取，未命中则执行 `f` 并回填缓存。
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

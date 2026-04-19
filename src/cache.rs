//! 缓存抽象层。
//!
//! 提供 [`CacheStore`] trait 和基于 `HashMap` 的内存实现 [`MemoryCache`]。
//! 生产环境可替换为 Redis 实现。

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

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

type CacheMap = std::collections::HashMap<String, (String, Option<tokio::time::Instant>)>;

/// 基于 `HashMap` 的内存缓存实现
///
/// 使用 `tokio::sync::RwLock` 保证并发安全，惰性清理过期条目。
/// 适用于开发环境和单实例部署。
#[derive(Clone)]
pub struct MemoryCache {
    inner: Arc<RwLock<CacheMap>>,
}

impl MemoryCache {
    /// 创建新的内存缓存
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(std::collections::HashMap::new())),
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
        let map = self.inner.read().await;
        let (value, deadline) = map.get(key)?;
        if let Some(dl) = deadline
            && tokio::time::Instant::now() > *dl
        {
            return None;
        }
        Some(value.clone())
    }

    async fn set(&self, key: &str, value: &str, ttl: Option<Duration>) -> AppResult<()> {
        let deadline = ttl.map(|d| tokio::time::Instant::now() + d);
        self.inner
            .write()
            .await
            .insert(key.to_string(), (value.to_string(), deadline));
        Ok(())
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.inner.write().await.remove(key);
        Ok(())
    }

    async fn delete_prefix(&self, prefix: &str) -> AppResult<u64> {
        let mut map = self.inner.write().await;
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        let count = keys.len() as u64;
        for key in keys {
            map.remove(&key);
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

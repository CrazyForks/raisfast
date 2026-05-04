//! IP 限流中间件
//!
//! 基于 sliding window 的请求限流，支持多命名限流器（全局、注册、登录、评论）。
//! 存储后端通过 [`RateLimitStore`] trait 抽象，当前提供 [`MemoryStore`] 实现，
//! 未来可扩展 Redis 后端以支持多实例水平扩展。

use crate::config::app::AppConfig;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::Extension;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;

/// 限流窗口配置。
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_requests: u32,
    pub window_secs: u64,
}

/// 单个限流键的计数窗口条目。
#[derive(Debug)]
struct Entry {
    count: u32,
    window_start: Instant,
}

/// 限流存储后端 trait。
///
/// 抽象限流计数器的读写接口，便于切换不同存储后端。
/// 当前仅实现 [`MemoryStore`]，未来可添加 Redis 实现。
#[async_trait::async_trait]
pub trait RateLimitStore: Send + Sync {
    /// 对指定键进行一次计数检查。
    ///
    /// 若当前窗口内未超过 `max_requests`，计数 +1 并返回 `true`；
    /// 否则返回 `false` 表示被限流。
    async fn check(&self, key: &str, config: &RateLimitConfig) -> bool;

    /// 清理过期的计数条目，释放内存。
    async fn cleanup_expired(&self, window_secs: u64);
}

/// 基于 `DashMap` 的分片锁内存存储实现。
///
/// 使用 `DashMap` 替代 `Mutex<HashMap>`，消除全局锁竞争。
/// 适用于高并发单实例部署；多实例部署应切换为 Redis 后端。
#[derive(Debug)]
pub struct MemoryStore {
    entries: DashMap<String, Entry>,
}

impl MemoryStore {
    /// 创建空的内存存储。
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl RateLimitStore for MemoryStore {
    async fn check(&self, key: &str, config: &RateLimitConfig) -> bool {
        let now = Instant::now();

        self.entries.retain(|_, entry| {
            now.duration_since(entry.window_start).as_secs() < config.window_secs * 2
        });

        let mut entry_ref = self.entries.entry(key.to_string()).or_insert(Entry {
            count: 0,
            window_start: now,
        });

        if now.duration_since(entry_ref.window_start).as_secs() >= config.window_secs {
            entry_ref.count = 0;
            entry_ref.window_start = now;
        }

        if entry_ref.count >= config.max_requests {
            return false;
        }

        entry_ref.count += 1;
        true
    }

    async fn cleanup_expired(&self, window_secs: u64) {
        let now = Instant::now();
        self.entries
            .retain(|_, entry| now.duration_since(entry.window_start).as_secs() < window_secs * 2);
    }
}

/// 限流器，组合存储后端与配置。
///
/// 通过泛型 [`RateLimitStore`] 支持不同存储后端。
#[derive(Debug)]
pub struct RateLimiter<S: RateLimitStore> {
    store: Arc<S>,
    config: RateLimitConfig,
}

impl<S: RateLimitStore> Clone for RateLimiter<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            config: self.config.clone(),
        }
    }
}

impl<S: RateLimitStore> RateLimiter<S> {
    /// 创建新的限流器。
    pub fn new(store: Arc<S>, config: RateLimitConfig) -> Self {
        Self { store, config }
    }

    /// 对指定键执行限流检查。
    pub async fn check(&self, key: &str) -> bool {
        self.store.check(key, &self.config).await
    }

    /// 清理过期条目。
    pub async fn cleanup_expired(&self) {
        self.store.cleanup_expired(self.config.window_secs).await;
    }
}

/// 命名限流器集合，通过 Extension 在路由间共享。
///
/// 每个限流器独立配置 `max_requests` / `window_secs`，
/// 共享同一个存储后端实例。
#[derive(Debug, Clone)]
pub struct RateLimiterSet {
    pub global: RateLimiter<MemoryStore>,
    pub register: RateLimiter<MemoryStore>,
    pub login: RateLimiter<MemoryStore>,
    pub comment: RateLimiter<MemoryStore>,
    pub api_token: RateLimiter<MemoryStore>,
}

impl RateLimiterSet {
    /// 根据应用配置创建限流器集合。
    #[must_use]
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            global: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: config.rate_limit_global_max,
                    window_secs: config.rate_limit_global_window,
                },
            ),
            register: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: config.rate_limit_register_max,
                    window_secs: config.rate_limit_register_window,
                },
            ),
            login: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: config.rate_limit_login_max,
                    window_secs: config.rate_limit_login_window,
                },
            ),
            comment: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: config.rate_limit_comment_max,
                    window_secs: config.rate_limit_comment_window,
                },
            ),
            api_token: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: config.rate_limit_api_token_max,
                    window_secs: config.rate_limit_api_token_window,
                },
            ),
        }
    }

    /// 创建包含默认配置的限流器集合（用于测试）。
    #[must_use]
    pub fn new_default() -> Self {
        Self {
            global: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: 60,
                    window_secs: 60,
                },
            ),
            register: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: 5,
                    window_secs: 3600,
                },
            ),
            login: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: 10,
                    window_secs: 60,
                },
            ),
            comment: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: 3,
                    window_secs: 60,
                },
            ),
            api_token: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: 120,
                    window_secs: 60,
                },
            ),
        }
    }
}

pub fn extract_client_ip(req: &Request) -> String {
    req.headers()
        .get("x-forwarded-for")
        .or_else(|| req.headers().get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0.ip().to_string())
        })
        .unwrap_or_default()
}

pub fn rate_limited_response() -> Response {
    crate::errors::app_error::AppError::TooManyRequests.into_response()
}

pub async fn global_rate_limit(
    Extension(limiters): Extension<RateLimiterSet>,
    req: Request,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);

    if !limiters.global.check(&ip).await {
        return rate_limited_response();
    }

    if let Some(prefix) = extract_api_token_prefix(&req)
        && !limiters.api_token.check(&format!("token:{prefix}")).await
    {
        return rate_limited_response();
    }

    next.run(req).await
}

pub fn extract_api_token_prefix(req: &Request) -> Option<String> {
    let auth = req.headers().get("authorization")?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    if token.starts_with("rblog_") {
        token.get(..12).map(|s| s.to_string())
    } else {
        None
    }
}

macro_rules! rate_limit_fn {
    ($name:ident, $specific:ident) => {
        pub async fn $name(
            axum::extract::Extension(limiters): axum::extract::Extension<RateLimiterSet>,
            req: Request,
            next: Next,
        ) -> Response {
            let ip = extract_client_ip(&req);

            if !limiters.global.check(&ip).await || !limiters.$specific.check(&ip).await {
                return rate_limited_response();
            }

            next.run(req).await
        }
    };
}

rate_limit_fn!(register_rate_limit, register);
rate_limit_fn!(login_rate_limit, login);
rate_limit_fn!(comment_rate_limit, comment);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let store = Arc::new(MemoryStore::new());
        let limiter = RateLimiter::new(
            store,
            RateLimitConfig {
                max_requests: 3,
                window_secs: 60,
            },
        );
        assert!(limiter.check("ip1").await);
        assert!(limiter.check("ip1").await);
        assert!(limiter.check("ip1").await);
    }

    #[tokio::test]
    async fn blocks_requests_over_limit() {
        let store = Arc::new(MemoryStore::new());
        let limiter = RateLimiter::new(
            store,
            RateLimitConfig {
                max_requests: 2,
                window_secs: 60,
            },
        );
        limiter.check("ip1").await;
        limiter.check("ip1").await;
        assert!(!limiter.check("ip1").await);
    }

    #[tokio::test]
    async fn different_keys_independent() {
        let store = Arc::new(MemoryStore::new());
        let limiter = RateLimiter::new(
            store,
            RateLimitConfig {
                max_requests: 1,
                window_secs: 60,
            },
        );
        limiter.check("ip1").await;
        assert!(limiter.check("ip2").await);
        assert!(!limiter.check("ip1").await);
    }

    #[tokio::test]
    async fn cleanup_removes_expired() {
        let store = Arc::new(MemoryStore::new());
        let config = RateLimitConfig {
            max_requests: 10,
            window_secs: 60,
        };
        {
            store.entries.insert(
                "old_key".to_string(),
                Entry {
                    count: 1,
                    window_start: Instant::now() - std::time::Duration::from_secs(200),
                },
            );
            store.entries.insert(
                "new_key".to_string(),
                Entry {
                    count: 1,
                    window_start: Instant::now(),
                },
            );
        }
        store.cleanup_expired(config.window_secs).await;
        assert!(!store.entries.contains_key("old_key"));
        assert!(store.entries.contains_key("new_key"));
    }
}

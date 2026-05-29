//! IP rate limiting middleware
//!
//! Sliding window request rate limiting, supporting multiple named limiters (global, register, login, comment).
//! Storage backend is abstracted via the [`RateLimitStore`] trait; currently provides a [`MemoryStore`]
//! implementation, with a Redis backend to be added in the future for multi-instance horizontal scaling.

use crate::config::app::AppConfig;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::Extension;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;

/// Rate limit window configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_requests: u32,
    pub window_secs: u64,
}

/// Counting window entry for a single rate limit key.
#[derive(Debug)]
struct Entry {
    count: u32,
    window_start: Instant,
}

/// Rate limit storage backend trait.
///
/// Abstracts the rate limit counter read/write interface for switching between storage backends.
/// Currently only [`MemoryStore`] is implemented; a Redis implementation can be added later.
#[async_trait::async_trait]
pub trait RateLimitStore: Send + Sync {
    /// Perform a counting check for the specified key.
    ///
    /// If the current window has not exceeded `max_requests`, increments the count and returns `true`;
    /// otherwise returns `false` indicating rate limited.
    async fn check(&self, key: &str, config: &RateLimitConfig) -> bool;

    /// Clean up expired count entries to release memory.
    async fn cleanup_expired(&self, window_secs: u64);
}

/// DashMap-based sharded lock memory store implementation.
///
/// Uses `DashMap` instead of `Mutex<HashMap>` to eliminate global lock contention.
/// Suitable for high-concurrency single-instance deployments; multi-instance deployments
/// should switch to a Redis backend.
#[derive(Debug)]
pub struct MemoryStore {
    entries: DashMap<String, Entry>,
}

impl MemoryStore {
    /// Create an empty memory store.
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

/// Rate limiter, combining a storage backend with configuration.
///
/// Supports different storage backends via the generic [`RateLimitStore`].
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
    /// Create a new rate limiter.
    pub fn new(store: Arc<S>, config: RateLimitConfig) -> Self {
        Self { store, config }
    }

    /// Perform a rate limit check for the specified key.
    pub async fn check(&self, key: &str) -> bool {
        self.store.check(key, &self.config).await
    }

    /// Clean up expired entries.
    pub async fn cleanup_expired(&self) {
        self.store.cleanup_expired(self.config.window_secs).await;
    }
}

/// Named rate limiter set, shared across routes via Extension.
///
/// Each limiter has independent `max_requests` / `window_secs` configuration,
/// sharing the same storage backend instance.
#[derive(Debug, Clone)]
pub struct RateLimiterSet {
    pub enabled: bool,
    pub global: RateLimiter<MemoryStore>,
    pub register: RateLimiter<MemoryStore>,
    pub login: RateLimiter<MemoryStore>,
    pub comment: RateLimiter<MemoryStore>,
    pub api_token: RateLimiter<MemoryStore>,
    pub payment_callback: RateLimiter<MemoryStore>,
}

impl RateLimiterSet {
    /// Create a rate limiter set from application configuration.
    #[must_use]
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            enabled: config.rate_limit_enabled,
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
            payment_callback: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: 30,
                    window_secs: 60,
                },
            ),
        }
    }

    /// Create a rate limiter set with default configuration (for testing).
    #[must_use]
    pub fn new_default() -> Self {
        Self {
            enabled: true,
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
            payment_callback: RateLimiter::new(
                Arc::new(MemoryStore::new()),
                RateLimitConfig {
                    max_requests: 30,
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
    crate::errors::app_error::AppError::TooManyRequests("rate limit exceeded".into())
        .into_response()
}

pub async fn global_rate_limit(
    Extension(limiters): Extension<RateLimiterSet>,
    req: Request,
    next: Next,
) -> Response {
    if !limiters.enabled {
        return next.run(req).await;
    }

    let path = req.uri().path();
    if path.starts_with("/api/v1/setup") {
        return next.run(req).await;
    }

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
    let auth = req
        .headers()
        .get(crate::constants::HEADER_AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = auth.strip_prefix(crate::constants::AUTH_BEARER_PREFIX)?;
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
            if !limiters.enabled {
                return next.run(req).await;
            }

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
rate_limit_fn!(payment_callback_rate_limit, payment_callback);

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

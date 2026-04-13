use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::Extension;
use axum::Json;
use axum::extract::{ConnectInfo, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_requests: u32,
    pub window_secs: u64,
}

#[derive(Debug)]
struct Entry {
    count: u32,
    window_start: Instant,
}

#[derive(Debug, Clone)]
pub struct RateLimiter {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    pub async fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut entries = self.entries.lock().await;

        entries.retain(|_, entry| {
            now.duration_since(entry.window_start).as_secs() < self.config.window_secs * 2
        });

        let entry = entries.entry(key.to_string()).or_insert(Entry {
            count: 0,
            window_start: now,
        });

        if now.duration_since(entry.window_start).as_secs() >= self.config.window_secs {
            entry.count = 0;
            entry.window_start = now;
        }

        if entry.count >= self.config.max_requests {
            return false;
        }

        entry.count += 1;
        true
    }

    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        let mut entries = self.entries.lock().await;
        entries.retain(|_, entry| {
            now.duration_since(entry.window_start).as_secs() < self.config.window_secs * 2
        });
    }
}

/// 命名限流器集合，通过 Extension 在路由间共享。
///
/// 每个限流器独立配置 `max_requests` / `window_secs`，
/// 避免宏中每次请求创建空实例的问题。
#[derive(Debug, Clone)]
pub struct RateLimiterSet {
    pub global: RateLimiter,
    pub register: RateLimiter,
    pub login: RateLimiter,
    pub comment: RateLimiter,
}

impl RateLimiterSet {
    /// 创建包含所有命名限流器的默认集合。
    pub fn new_default() -> Self {
        Self {
            global: RateLimiter::new(RateLimitConfig {
                max_requests: 60,
                window_secs: 60,
            }),
            register: RateLimiter::new(RateLimitConfig {
                max_requests: 5,
                window_secs: 3600,
            }),
            login: RateLimiter::new(RateLimitConfig {
                max_requests: 10,
                window_secs: 60,
            }),
            comment: RateLimiter::new(RateLimitConfig {
                max_requests: 3,
                window_secs: 60,
            }),
        }
    }
}

fn extract_client_ip(req: &Request) -> String {
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

fn rate_limited_response() -> Response {
    let locale = crate::middleware::locale::current_locale();
    rust_i18n::set_locale(&locale);
    let message = rust_i18n::t!("errors.too_many_requests");
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({
            "code": 42900,
            "message": message,
            "data": null
        })),
    )
        .into_response()
}

pub async fn global_rate_limit(
    Extension(limiters): Extension<RateLimiterSet>,
    req: Request,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);

    if limiters.global.check(&ip).await {
        next.run(req).await
    } else {
        rate_limited_response()
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

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

pub async fn global_rate_limit(
    Extension(limiter): axum::extract::Extension<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);

    if limiter.check(&ip).await {
        next.run(req).await
    } else {
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
}

macro_rules! rate_limit_fn {
    ($name:ident, $max:expr, $window:expr) => {
        pub async fn $name(
            axum::extract::Extension(global): axum::extract::Extension<RateLimiter>,
            req: Request,
            next: Next,
        ) -> Response {
            let ip = extract_client_ip(&req);
            let specific = RateLimiter::new(RateLimitConfig {
                max_requests: $max,
                window_secs: $window,
            });

            if !global.check(&ip).await || !specific.check(&ip).await {
                let locale = crate::middleware::locale::current_locale();
                rust_i18n::set_locale(&locale);
                let message = rust_i18n::t!("errors.too_many_requests");
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({
                        "code": 42900,
                        "message": message,
                        "data": null
                    })),
                )
                    .into_response();
            }

            next.run(req).await
        }
    };
}

rate_limit_fn!(register_rate_limit, 5, 3600);
rate_limit_fn!(login_rate_limit, 10, 60);
rate_limit_fn!(comment_rate_limit, 3, 60);

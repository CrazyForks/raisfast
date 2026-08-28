//! Ingress endpoints — the external face of the plane (integration.md §11).
//!
//! `GET  /api/v1/ingress/{channel_key}` — challenge handshake (WeChat style)
//! `POST /api/v1/ingress/{channel_key}` — push callback entry
//!
//! No JWT here by design: trust comes from L0 verification (signature/token)
//! plus a per-channel rate limit. Routes register as `public` for the
//! permission guard.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::integration::channel::ItgChannel;
use crate::integration::pipeline::{AckAction, ReplayOutcome};
use crate::integration::verify::{InboundHttpRequest, VerifyOutcome};
use crate::middleware::auth::{AuthUser, TokenAction};
use crate::AppState;

/// POST /admin/integration/receipts/{id}/replay — re-run the route from the
/// stored envelope snapshot (§6.4). Modes: `upsert` (default) / `dry-run`.
pub async fn replay(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Update)?;
    let Some(plane) = state.integration.as_ref() else {
        return Err(AppError::Internal(anyhow::anyhow!("integration plane disabled")));
    };
    let trace_id = crate::types::snowflake_id::parse_id(&id)?;
    let dry_run = body
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .map(|m| m == "dry-run")
        .unwrap_or(false);

    let outcome = plane
        .pipeline()
        .run_replay(trace_id, dry_run)
        .await?;
    let data = match outcome {
        ReplayOutcome::Upserted { target_id } => serde_json::json!({
            "replayed": true,
            "mode": "upsert",
            "target_id": target_id.map(|t| t.to_string()),
        }),
        ReplayOutcome::DryRun { report } => serde_json::json!({
            "replayed": false,
            "mode": "dry-run",
            "report": report,
        }),
    };
    Ok(ApiResponse::success(data))
}

/// Per-channel fixed-window limiter (configurable via
/// `channel.backpressure.per_second`; default 100 req/min). Created lazily
/// on first request per channel key.
#[derive(Default)]
pub struct IngressRateLimiter {
    inner: dashmap::DashMap<
        String,
        crate::middleware::rate_limit::RateLimiter<
            crate::middleware::rate_limit::MemoryStore,
        >,
    >,
}

const DEFAULT_PER_MINUTE: u32 = 100;

impl IngressRateLimiter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check-and-consume one request slot for the channel.
    #[must_use]
    pub async fn allow(&self, channel: &ItgChannel) -> bool {
        let per_minute = channel
            .backpressure
            .as_ref()
            .and_then(|b| b.get("per_second"))
            .and_then(serde_json::Value::as_u64)
            .map_or(DEFAULT_PER_MINUTE, |ps| (ps * 60).min(u64::from(u32::MAX)) as u32);
        // Window 60s with `per_minute` budget — mirrors RateLimiter::new(min, window).
        let limiter = self
            .inner
            .entry(channel.channel_key.clone())
            .or_insert_with(|| {
                crate::middleware::rate_limit::RateLimiter::new(
                    std::sync::Arc::new(crate::middleware::rate_limit::MemoryStore::new()),
                    crate::middleware::rate_limit::RateLimitConfig {
                        max_requests: per_minute,
                        window_secs: 60,
                    },
                )
            });
        limiter.check(&channel.channel_key).await
    }
}

fn ingress_disabled() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "integration plane disabled",
    )
        .into_response()
}

async fn resolve_channel(
    state: &AppState,
    channel_key: &str,
) -> Result<std::sync::Arc<ItgChannel>, Response> {
    let Some(plane) = state.integration.as_ref() else {
        return Err(ingress_disabled());
    };
    match plane
        .channels()
        .get(crate::constants::DEFAULT_TENANT, channel_key)
        .await
    {
        Ok(ch) => Ok(ch),
        Err(_) => Err((StatusCode::NOT_FOUND, "unknown channel").into_response()),
    }
}

/// GET — challenge handshake verification.
pub async fn challenge(
    State(state): State<AppState>,
    Path(channel_key): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let ch = match resolve_channel(&state, &channel_key).await {
        Ok(ch) => ch,
        Err(resp) => return resp,
    };

    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let req = InboundHttpRequest {
        method: "GET".into(),
        query,
        headers: Vec::new(),
        body: Vec::new(),
    };

    let vault = state.integration.as_ref().and_then(|p| p.vault());
    match crate::integration::verify::verify(&ch, vault, &req) {
        VerifyOutcome::ChallengeEcho(echo) => (StatusCode::OK, echo).into_response(),
        VerifyOutcome::Ok => {
            // Channel without a GET flow (e.g. hmac) — nothing to handshake.
            (StatusCode::METHOD_NOT_ALLOWED, "no GET flow").into_response()
        }
        VerifyOutcome::Reject { status, reason } => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::UNAUTHORIZED);
            (code, reason).into_response()
        }
    }
}

/// POST — push callback entry.
pub async fn push(
    State(state): State<AppState>,
    Path(channel_key): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(plane) = state.integration.as_ref() else {
        return ingress_disabled();
    };
    let ch = match plane
        .channels()
        .get(crate::constants::DEFAULT_TENANT, &channel_key)
        .await
    {
        Ok(ch) => ch,
        Err(_) => return (StatusCode::NOT_FOUND, "unknown channel").into_response(),
    };

    if !plane.limiter().allow(&ch).await {
        tracing::warn!(channel = %channel_key, "ingress rate limited");
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }

    let limit = plane.body_limit();
    if body.len() > limit {
        return (StatusCode::PAYLOAD_TOO_LARGE, "body exceeds limit").into_response();
    }

    let req = InboundHttpRequest {
        method: "POST".into(),
        query: String::new(),
        headers: headers
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|val| (k.as_str().to_ascii_lowercase(), val.to_string()))
            })
            .collect(),
        body: body.to_vec(),
    };

    let outcome = plane.pipeline().run_push(&ch, &req).await;
    match outcome.ack {
        AckAction::Http { status, body } => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            match body {
                Some(b) => (code, b).into_response(),
                None => (code, "").into_response(),
            }
        }
    }
}

/// Route registration (public — no JWT; trust is L0 verification).
pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    _config: &crate::config::app::AppConfig,
) -> axum::Router<AppState> {
    let r = axum::Router::new();
    let r = crate::reg_route!(
        r,
        registry,
        false, // ingress is machine-facing: no RESTful aliasing
        "/ingress/{channel_key}",
        get,
        challenge,
        "integration",
        "ingress",
        "public"
    );
    let r = crate::reg_route!(
        r,
        registry,
        false,
        "/ingress/{channel_key}",
        post,
        push,
        "integration",
        "ingress",
        "public"
    );
    let r = crate::reg_route!(r, registry, false, "/admin/integration/receipts/{id}/replay", post, replay, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/receipts", get, crate::integration::admin::list_receipts, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/receipts/{id}", get, crate::integration::admin::get_receipt, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/receipts/{id}/trace", get, crate::integration::admin::get_trace, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/channels", get, crate::integration::admin::list_channels, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/channels", post, crate::integration::admin::create_channel, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/channels/{id}", get, crate::integration::admin::get_channel, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/channels/{id}", put, crate::integration::admin::update_channel, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/channels/{id}", delete, crate::integration::admin::delete_channel, "integration", "admin/integration");
    let r = crate::reg_route!(r, registry, false, "/admin/integration/channels/{id}/test-mapping", post, crate::integration::admin::test_mapping, "integration", "admin/integration");
    crate::reg_route!(r, registry, false, "/admin/integration/channels/{id}/test-connection", post, crate::integration::admin::test_connection, "integration", "admin/integration")
}

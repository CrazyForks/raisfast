//! Audit-on-deny middleware
//!
//! Inspects every API response; when the status is **403 Forbidden** or
//! **401 Unauthorized**, records a persistent audit-log entry identifying
//! *who* was denied, *what* they tried to do, and *from where*.
//!
//! Identity resolution is deferred — the bearer token is captured cheaply
//! on every request (a header read), but the full token verification only
//! runs when the response is actually a denial, keeping the happy path
//! zero-cost.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;

use crate::AppState;

/// Paths that never produce audit-worthy denials (health checks, metrics).
const SKIP_PATHS: &[&str] = &["/health", "/healthz", "/readyz", "/metrics", "/feed.xml"];

/// Capture the bearer token + request metadata before dispatch, then audit-log
/// any 401/403 response.
pub async fn audit_denied_layer(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let path = req.uri().path();
    if SKIP_PATHS.contains(&path) {
        return next.run(req).await;
    }

    let method = req.method().clone();
    let path_owned = path.to_string();

    let bearer = req
        .headers()
        .get(crate::constants::HEADER_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix(crate::constants::AUTH_BEARER_PREFIX))
        .map(str::to_string);

    let ip = req
        .headers()
        .get("x-forwarded-for")
        .or_else(|| req.headers().get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or("").trim().to_string());

    let user_agent = req
        .headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let response = next.run(req).await;
    let status = response.status().as_u16();

    if status == 403 || status == 401 {
        let (actor_id, actor_role, tenant_id) = match &bearer {
            Some(token) => match crate::middleware::auth::resolve_bearer(token, &state).await {
                Some(c) => (
                    Some(*c.user_id),
                    Some(
                        c.roles
                            .iter()
                            .copied()
                            .map(crate::models::user::UserRole::as_str)
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                    c.tenant_id,
                ),
                None => (None, None, crate::constants::DEFAULT_TENANT.to_string()),
            },
            None => (None, None, crate::constants::DEFAULT_TENANT.to_string()),
        };

        let action = if status == 403 {
            "permission_denied"
        } else {
            "authentication_failed"
        };
        let detail = format!("{} {}", method, path_owned);

        if let Err(e) = state
            .audit
            .log(
                &tenant_id,
                actor_id,
                actor_role.as_deref(),
                action,
                "request",
                Some(&path_owned),
                Some(&detail),
                ip.as_deref(),
                user_agent.as_deref(),
            )
            .await
        {
            tracing::warn!("audit denied request failed: {e}");
        }
    }

    response
}

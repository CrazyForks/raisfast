//! Subdomain-based tenant resolution middleware.
//!
//! When `BUILTIN_TENANTABLE=true` and `BASE_DOMAIN` is configured, this middleware:
//! 1. Extracts the `Host` header from the request
//! 2. Parses the subdomain portion (e.g., `tenant1.app.com` → `"tenant1"`)
//! 3. Looks up the tenant by domain in the `tenants` table
//! 4. Injects the resolved `tenant_id` as `X-Tenant-ID` header
//!
//! This runs BEFORE auth extraction so `AuthUser` sees the resolved tenant.

use axum::extract::State;
use axum::http::{HeaderName, HeaderValue, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::AppState;

static X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

pub async fn subdomain_tenant_resolver(
    State(state): State<AppState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if state.config.builtin_tenantable
        && let Some(base_domain) = &state.config.base_domain
        && let Some(host) = request.headers().get("host").and_then(|v| v.to_str().ok())
    {
        let host = host.split(':').next().unwrap_or(host);
        if let Some(subdomain) = extract_subdomain(host, base_domain) {
            let tenant_id = resolve_tenant(&state, &subdomain).await;
            if let Some(tid) = tenant_id {
                request.headers_mut().insert(
                    X_TENANT_ID.clone(),
                    HeaderValue::from_str(&tid)
                        .unwrap_or_else(|_| HeaderValue::from_static("default")),
                );
            }
        }
    }
    next.run(request).await
}

fn extract_subdomain(host: &str, base_domain: &str) -> Option<String> {
    let host = host.split(':').next().unwrap_or(host);
    if !host.ends_with(base_domain) {
        return None;
    }
    let prefix = host.strip_suffix(base_domain)?.strip_suffix('.')?;
    if prefix.is_empty() {
        return None;
    }
    Some(prefix.to_string())
}

async fn resolve_tenant(state: &AppState, subdomain: &str) -> Option<String> {
    match state.tenant.get_by_domain(subdomain).await {
        Ok(Some(tenant)) => Some(tenant.name.clone()),
        Ok(None) => {
            tracing::debug!("no tenant found for subdomain: {subdomain}");
            None
        }
        Err(e) => {
            tracing::warn!("error resolving tenant for subdomain {subdomain}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_subdomain_valid() {
        assert_eq!(
            extract_subdomain("tenant1.app.com", "app.com"),
            Some("tenant1".to_string())
        );
    }

    #[test]
    fn extract_subdomain_no_subdomain() {
        assert_eq!(extract_subdomain("app.com", "app.com"), None);
    }

    #[test]
    fn extract_subdomain_wrong_domain() {
        assert_eq!(extract_subdomain("tenant1.other.com", "app.com"), None);
    }

    #[test]
    fn extract_subdomain_nested() {
        assert_eq!(
            extract_subdomain("a.b.app.com", "app.com"),
            Some("a.b".to_string())
        );
    }

    #[test]
    fn extract_subdomain_with_port() {
        assert_eq!(
            extract_subdomain("tenant1.app.com:9898", "app.com"),
            Some("tenant1".to_string())
        );
    }
}

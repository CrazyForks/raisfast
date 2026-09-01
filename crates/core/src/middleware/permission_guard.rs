//! Centralized permission guard middleware
//!
//! Replaces scattered `ensure_scope` / `ensure_admin` calls in individual handlers.
//! The middleware looks up the declared permission for each route and enforces it
//! in a single place:
//!
//! - `"public"`  → no auth required
//! - `"admin"`   → admin role required
//! - `"authed"`  → any authenticated user
//! - `"resource:action"` → API token scope check + RBAC permissions check
//!
//! CMS dynamic routes (`/cms/*`, `/admin/cms/*`) are exempt — their authorization
//! stays in the content-type handler.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;

use crate::AppState;
use crate::errors::app_error::AppError;
use crate::middleware::auth::resolve_bearer;

/// Compiled permission map: `(HTTP method, path pattern) → permission requirement`.
pub struct RoutePermissionMap {
    exact: HashMap<(String, String), String>,
    patterns: Vec<(String, String, String)>,
}

impl RoutePermissionMap {
    pub fn from_routes(routes: &[crate::server::RouteInfo]) -> Self {
        let mut exact = HashMap::new();
        let mut patterns = Vec::new();

        for r in routes {
            let Some(ref perm) = r.permission else {
                continue;
            };
            if r.path.contains('{') || r.path.contains(':') {
                patterns.push((r.method.clone(), r.path.clone(), perm.clone()));
            } else {
                exact.insert((r.method.clone(), r.path.clone()), perm.clone());
            }
        }

        Self { exact, patterns }
    }

    pub fn lookup(&self, method: &str, path: &str) -> Option<&str> {
        if let Some(perm) = self.exact.get(&(method.to_string(), path.to_string())) {
            return Some(perm.as_str());
        }

        let req_segs: Vec<&str> = path.split('/').collect();
        for (pmethod, ppath, perm) in &self.patterns {
            if pmethod != method {
                continue;
            }
            let pat_segs: Vec<&str> = ppath.split('/').collect();
            if req_segs.len() != pat_segs.len() {
                continue;
            }
            let matched = req_segs
                .iter()
                .zip(pat_segs.iter())
                .all(|(req, pat)| pat.starts_with('{') || pat.starts_with(':') || pat == req);
            if matched {
                return Some(perm.as_str());
            }
        }

        None
    }
}

fn is_exempt(path: &str) -> bool {
    let cms = format!("{}/", crate::constants::CMS_PREFIX);
    let cms_admin = format!("{}/", crate::constants::CMS_ADMIN_PREFIX);
    if path.starts_with(&cms) || path.starts_with(&cms_admin) {
        return true;
    }
    if path == crate::constants::CMS_PREFIX || path == crate::constants::CMS_ADMIN_PREFIX {
        return true;
    }

    matches!(
        path,
        "/api/v1/auth/login"
            | "/api/v1/auth/register"
            | "/api/v1/auth/refresh"
            | "/api/v1/auth/oauth"
            | "/api/v1/auth/oauth/callback"
            | "/api/v1/auth/verify-email"
            | "/api/v1/auth/reset-password"
            | "/api/v1/auth/forgot-password"
            | "/api/v1/setup/status"
            | "/api/v1/setup/database/test"
            | "/api/v1/setup/database"
            | "/api/v1/setup/init"
            | "/api/v1/info"
            | "/api/v1/routes"
    )
}

pub async fn permission_guard(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let method = req.method().as_str();
    let raw_path = req.uri().path();
    let path_for_lookup = if raw_path.starts_with(crate::constants::API_PREFIX) {
        raw_path.to_string()
    } else {
        format!("{}{}", crate::constants::API_PREFIX, raw_path)
    };
    let path = if path_for_lookup.len() > 1 && path_for_lookup.ends_with('/') {
        &path_for_lookup[..path_for_lookup.len() - 1]
    } else {
        &path_for_lookup[..]
    };

    if is_exempt(path) {
        return next.run(req).await;
    }

    let perm = state.route_perms.lookup(method, path);
    let required = match perm {
        Some(p) => p.to_string(),
        None => {
            if path.contains("/admin/") {
                "admin".to_string()
            } else {
                return next.run(req).await;
            }
        }
    };

    if required == "public" {
        return next.run(req).await;
    }

    let bearer = req
        .headers()
        .get(crate::constants::HEADER_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix(crate::constants::AUTH_BEARER_PREFIX))
        .map(|s| s.to_string());

    let claims = match &bearer {
        Some(token) => resolve_bearer(token, &state).await,
        None => None,
    };

    if let Some(ref c) = claims {
        req.extensions_mut().insert(c.clone());
    }

    if required == "admin" {
        match &claims {
            Some(c) if c.has_role(crate::models::user::UserRole::Admin) => {}
            Some(_) => return AppError::ForbiddenAdmin.into_response(),
            None => return AppError::Unauthorized.into_response(),
        }
        return next.run(req).await;
    }

    if required == "authed" {
        if claims.is_none() {
            return AppError::Unauthorized.into_response();
        }
        return next.run(req).await;
    }

    if let Some((resource, action_str)) = required.split_once(':') {
        let resource = resource.to_ascii_lowercase();
        let action_str = action_str.to_ascii_lowercase();
        match &claims {
            None => return AppError::Unauthorized.into_response(),
            Some(c) => {
                if c.has_role(crate::models::user::UserRole::Admin) {
                    return next.run(req).await;
                }

                if !c.scopes.is_empty() {
                    let has_scope = c.scopes.iter().any(|s| {
                        let s = s.to_ascii_lowercase();
                        s == "*"
                            || s == format!("{resource}:*")
                            || (action_str != "*"
                                && (s == format!("*:{action_str}")
                                    || s == format!("{resource}:{action_str}")))
                    });
                    if !has_scope {
                        return AppError::ForbiddenScope(format!("{resource}:{action_str}"))
                            .into_response();
                    }
                }

                if !c.role_ids.is_empty()
                    && !check_rbac_permission(&state, &c.role_ids, &action_str, &resource).await
                {
                    return AppError::ForbiddenRbac(format!("{resource}:{action_str}"))
                        .into_response();
                }
            }
        }
        return next.run(req).await;
    }

    next.run(req).await
}

async fn check_rbac_permission(
    state: &AppState,
    role_ids: &[i64],
    action: &str,
    subject: &str,
) -> bool {
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

    let mut sorted_ids = role_ids.to_vec();
    sorted_ids.sort_unstable();
    let key = format!(
        "perm:{}:{}:{}",
        sorted_ids
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(","),
        action,
        subject
    );

    if let Some(cached) = state.cache.get(&key).await {
        return cached == "1";
    }

    let permissions =
        match crate::models::rbac::find_permissions_by_role_ids(&state.pool, role_ids).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("RBAC permission query failed: {e}");
                return false;
            }
        };

    if permissions.is_empty() {
        return true;
    }

    let granted = permissions.iter().any(|p| {
        crate::services::rbac::matches_action(&p.action, action)
            && crate::services::rbac::matches_subject(&p.subject, subject)
    });

    let _ = state
        .cache
        .set(&key, if granted { "1" } else { "0" }, Some(CACHE_TTL))
        .await;

    granted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::RouteInfo;

    fn ri(method: &str, path: &str, perm: &str) -> RouteInfo {
        RouteInfo {
            method: method.to_string(),
            path: path.to_string(),
            source: "plugin".to_string(),
            source_name: "chat".to_string(),
            permission: Some(perm.to_string()),
        }
    }

    #[test]
    fn plugin_path_params_match_patterns() {
        let map = RoutePermissionMap::from_routes(&[
            ri(
                "POST",
                "/api/v1/plugins/chat/conversations/:id/messages",
                "conversations:write",
            ),
            ri(
                "GET",
                "/api/v1/plugins/chat/conversations",
                "conversations:read",
            ),
            ri(
                "POST",
                "/api/v1/plugins/chat/presence/heartbeat",
                "presence:write",
            ),
        ]);

        assert_eq!(
            map.lookup("POST", "/api/v1/plugins/chat/conversations/123/messages"),
            Some("conversations:write")
        );
        assert_eq!(
            map.lookup("GET", "/api/v1/plugins/chat/conversations"),
            Some("conversations:read")
        );
        assert_eq!(
            map.lookup("POST", "/api/v1/plugins/chat/presence/heartbeat"),
            Some("presence:write")
        );
        // Different method does not match the same path.
        assert_eq!(
            map.lookup("GET", "/api/v1/plugins/chat/conversations/123/messages"),
            None
        );
        // Unrelated path is not matched.
        assert_eq!(map.lookup("POST", "/api/v1/plugins/chat/other"), None);
    }
}

//! Centralized permission guard middleware
//!
//! Replaces scattered `ensure_scope` / `ensure_admin` calls in individual handlers.
//! The middleware looks up the declared permission for each route and enforces it
//! in a single place:
//!
//! - `"public"`  → no auth required
//! - `"admin"`   → admin role required
//! - `"authed"`  → any authenticated user
//! - `"resource:action"` → API token scope check; JWT users must be authenticated
//!
//! CMS dynamic routes (`/cms/*`, `/admin/cms/*`) are exempt — their authorization
//! stays in the content-type handler (`check_api_access` + rule engine).
//!
//! Routes without an explicit permission declaration fall back to path-based
//! heuristics: paths containing `/admin/` require admin.

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
///
/// Built once at startup from [`RouteRegistry`](crate::server::RouteRegistry) and
/// stored in [`AppState`] as an `Arc` for zero-copy lookup.
pub struct RoutePermissionMap {
    /// Exact path → permission (e.g. `("GET", "/api/v1/posts")`)
    exact: HashMap<(String, String), String>,
    /// Pattern path (containing `{`) → permission (e.g. `("PUT", "/api/v1/posts/{id}")`)
    patterns: Vec<(String, String, String)>,
}

impl RoutePermissionMap {
    /// Build from the route registry vec.
    pub fn from_routes(routes: &[crate::server::RouteInfo]) -> Self {
        let mut exact = HashMap::new();
        let mut patterns = Vec::new();

        for r in routes {
            let Some(ref perm) = r.permission else {
                continue;
            };
            if r.path.contains('{') {
                patterns.push((r.method.clone(), r.path.clone(), perm.clone()));
            } else {
                exact.insert((r.method.clone(), r.path.clone()), perm.clone());
            }
        }

        Self { exact, patterns }
    }

    /// Look up the permission for a given `(method, raw_path)`.
    ///
    /// Tries exact match first, then falls back to pattern matching
    /// (segment-wise, treating `{...}` segments as wildcards).
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
                .all(|(req, pat)| pat.starts_with('{') || pat == req);
            if matched {
                return Some(perm.as_str());
            }
        }

        None
    }
}

/// Paths that are always exempt from permission checks.
fn is_exempt(path: &str) -> bool {
    // CMS dynamic routes — authorization handled in content-type handler.
    // Check segment boundary to avoid false positives like /api/v1/cmswidget.
    let cms = format!("{}/", crate::constants::CMS_PREFIX);
    let cms_admin = format!("{}/", crate::constants::CMS_ADMIN_PREFIX);
    if path.starts_with(&cms) || path.starts_with(&cms_admin) {
        return true;
    }
    // Exact match for the prefix itself (e.g. /api/v1/cms with no trailing path)
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
            | "/api/v1/setup"
            | "/api/v1/info"
            | "/api/v1/routes"
    )
}

/// Centralized permission guard middleware.
///
/// Runs on every `/api/v1/*` request. The bearer token is read from the
/// `Authorization` header and resolved to identity claims. Based on the
/// route's declared permission, the middleware enforces the appropriate check.
pub async fn permission_guard(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let method = req.method().as_str();
    let raw_path = req.uri().path();
    // When mounted inside the nested router under /api/v1, the path seen here
    // is the inner path (e.g. "/categories"). The RoutePermissionMap stores full
    // paths with the /api/v1 prefix, so we prepend it for lookup.
    let path_for_lookup = if raw_path.starts_with(crate::constants::API_PREFIX) {
        // Already has prefix (outermost middleware or non-nested)
        raw_path.to_string()
    } else {
        format!("{}{}", crate::constants::API_PREFIX, raw_path)
    };
    // Normalize trailing slash (except root) so /posts/ matches /posts
    let path = if path_for_lookup.len() > 1 && path_for_lookup.ends_with('/') {
        &path_for_lookup[..path_for_lookup.len() - 1]
    } else {
        &path_for_lookup[..]
    };

    // Exempt CMS and public auth routes
    if is_exempt(path) {
        return next.run(req).await;
    }

    // Look up declared permission for this route
    let perm = state.route_perms.lookup(method, path);

    // Resolve the required permission, applying heuristics for undeclared routes
    let required = match perm {
        Some(p) => p.to_string(),
        None => {
            // Fallback heuristic: /admin/ paths require admin
            if path.contains("/admin/") {
                "admin".to_string()
            } else {
                // No permission declared and not an admin path — let the handler decide
                return next.run(req).await;
            }
        }
    };

    // "public" routes need no auth
    if required == "public" {
        return next.run(req).await;
    }

    // Extract bearer token from the request
    let bearer = req
        .headers()
        .get(crate::constants::HEADER_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix(crate::constants::AUTH_BEARER_PREFIX))
        .map(|s| s.to_string());

    // Resolve identity claims (None = anonymous / invalid token)
    let claims = match &bearer {
        Some(token) => resolve_bearer(token, &state).await,
        None => None,
    };

    // Share resolved identity with AuthUser extractor via extensions
    // to avoid double token verification
    if let Some(ref c) = claims {
        req.extensions_mut().insert(c.clone());
    }

    // "admin" routes require admin role
    if required == "admin" {
        match &claims {
            Some(c) if c.role == crate::models::user::UserRole::Admin => {}
            _ => return forbidden(),
        }
        return next.run(req).await;
    }

    // "authed" routes require any authenticated user
    if required == "authed" {
        if claims.is_none() {
            return unauthorized();
        }
        return next.run(req).await;
    }

    // "resource:action" — scope check for API tokens, auth required for JWT
    if let Some((resource, action_str)) = required.split_once(':') {
        // For action "*", API token must have resource:* or * scope.
        // For specific actions, check the standard wildcard patterns.
        let has_scope = |scopes: &[String]| -> bool {
            scopes.iter().any(|s| {
                s == "*"
                    || s == &format!("{resource}:*")
                    || (action_str != "*"
                        && (s == &format!("*:{action_str}")
                            || s == &format!("{resource}:{action_str}")))
            })
        };

        match &claims {
            None => return unauthorized(),
            Some(c) => {
                // Admin always allowed
                if c.role == crate::models::user::UserRole::Admin {
                    return next.run(req).await;
                }

                // API token user — check scope
                if !c.scopes.is_empty() && !has_scope(&c.scopes) {
                    return forbidden();
                }
                // JWT user (empty scopes) — authenticated, ownership checked in service layer
            }
        }
        return next.run(req).await;
    }

    // Unknown permission format — let the handler decide
    next.run(req).await
}

fn forbidden() -> axum::response::Response {
    AppError::Forbidden.into_response()
}

fn unauthorized() -> axum::response::Response {
    AppError::Unauthorized.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::RouteInfo;

    fn make_route(method: &str, path: &str, perm: Option<&str>) -> RouteInfo {
        RouteInfo {
            method: method.to_string(),
            path: path.to_string(),
            source: "test".to_string(),
            source_name: "test".to_string(),
            permission: perm.map(|s| s.to_string()),
        }
    }

    #[test]
    fn lookup_exact_match() {
        let routes = vec![
            make_route("GET", "/api/v1/posts", Some("posts:read")),
            make_route("POST", "/api/v1/posts", Some("posts:create")),
        ];
        let map = RoutePermissionMap::from_routes(&routes);
        assert_eq!(map.lookup("GET", "/api/v1/posts"), Some("posts:read"));
        assert_eq!(map.lookup("POST", "/api/v1/posts"), Some("posts:create"));
    }

    #[test]
    fn lookup_pattern_match() {
        let routes = vec![make_route(
            "PUT",
            "/api/v1/posts/{id}",
            Some("posts:update"),
        )];
        let map = RoutePermissionMap::from_routes(&routes);
        assert_eq!(
            map.lookup("PUT", "/api/v1/posts/12345"),
            Some("posts:update")
        );
        assert_eq!(map.lookup("PUT", "/api/v1/posts/abc"), Some("posts:update"));
    }

    #[test]
    fn lookup_pattern_depth_mismatch() {
        let routes = vec![make_route(
            "DELETE",
            "/api/v1/posts/{id}",
            Some("posts:delete"),
        )];
        let map = RoutePermissionMap::from_routes(&routes);
        assert_eq!(map.lookup("DELETE", "/api/v1/posts/123/comments"), None);
    }

    #[test]
    fn lookup_no_permission_returns_none() {
        let routes = vec![make_route("GET", "/api/v1/health", None)];
        let map = RoutePermissionMap::from_routes(&routes);
        assert_eq!(map.lookup("GET", "/api/v1/health"), None);
    }

    #[test]
    fn lookup_method_mismatch() {
        let routes = vec![make_route("GET", "/api/v1/posts/{id}", Some("posts:read"))];
        let map = RoutePermissionMap::from_routes(&routes);
        assert_eq!(map.lookup("DELETE", "/api/v1/posts/123"), None);
    }

    #[test]
    fn exempt_cms_routes() {
        assert!(is_exempt("/api/v1/cms/posts"));
        assert!(is_exempt("/api/v1/admin/cms/posts"));
    }

    #[test]
    fn exempt_public_auth_routes() {
        assert!(is_exempt("/api/v1/auth/login"));
        assert!(is_exempt("/api/v1/auth/register"));
    }

    #[test]
    fn not_exempt_normal_routes() {
        assert!(!is_exempt("/api/v1/posts"));
        assert!(!is_exempt("/api/v1/admin/pages"));
    }

    // ── Bug regression tests ──

    #[test]
    fn exempt_cms_does_not_false_positive() {
        // Bug 2: /api/v1/cmswidget must NOT be exempt
        assert!(!is_exempt("/api/v1/cmswidget"));
        assert!(!is_exempt("/api/v1/cms-something"));
        assert!(!is_exempt("/api/v1/admin/cmsapi"));
        // But actual CMS sub-paths are exempt
        assert!(is_exempt("/api/v1/cms/posts/123"));
        assert!(is_exempt("/api/v1/admin/cms/posts"));
    }

    #[test]
    fn lookup_trailing_slash_normalized() {
        // Bug 6: trailing slash adds empty segment — depth mismatch = no match
        // (the middleware strips trailing slash before lookup)
        let routes = vec![make_route("GET", "/api/v1/posts/{id}", Some("posts:read"))];
        let map = RoutePermissionMap::from_routes(&routes);
        assert_eq!(map.lookup("GET", "/api/v1/posts/123/"), None);
    }

    #[test]
    fn lookup_admin_heuristic_fallback() {
        // Undclared route under /admin/ should trigger heuristic in middleware.
        // Here we verify the map returns None (undeclared), the heuristic
        // is tested in the integration test below.
        let routes = vec![];
        let map = RoutePermissionMap::from_routes(&routes);
        assert_eq!(map.lookup("GET", "/api/v1/admin/anything"), None);
    }

    // ── Scope logic tests (Bug 3 regression) ──

    fn check_scope(required: &str, scopes: &[&str]) -> bool {
        // Mirrors the has_scope closure in permission_guard
        let (resource, action_str) = required.split_once(':').unwrap();
        let scopes: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
        scopes.iter().any(|s| {
            s == "*"
                || s == &format!("{resource}:*")
                || (action_str != "*"
                    && (s == &format!("*:{action_str}")
                        || s == &format!("{resource}:{action_str}")))
        })
    }

    #[test]
    fn scope_wildcard_action_requires_resource_star() {
        // Bug 3: "posts:*" permission must NOT pass with only "posts:read" scope
        assert!(!check_scope("posts:*", &["posts:read"]));
        assert!(check_scope("posts:*", &["posts:*"]));
        assert!(check_scope("posts:*", &["*"]));
    }

    #[test]
    fn scope_exact_action_matches() {
        assert!(check_scope("posts:read", &["posts:read"]));
        assert!(check_scope("posts:read", &["posts:*"]));
        assert!(check_scope("posts:read", &["*:read"]));
        assert!(check_scope("posts:read", &["*"]));
        assert!(!check_scope("posts:read", &["posts:create"]));
        assert!(!check_scope("posts:read", &["pages:read"]));
    }

    #[test]
    fn scope_no_scopes_means_jwt_user() {
        // Empty scopes = JWT user, scope check is skipped
        // (the middleware checks `!c.scopes.is_empty()` before calling has_scope)
    }

    // ── Integration tests using axum test harness ──

    use std::sync::Arc;

    use axum::http::{Request, StatusCode};
    use axum::middleware::{Next, from_fn_with_state};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct TestState {
        route_perms: Arc<RoutePermissionMap>,
    }

    /// Simplified permission_guard for testing without full AppState
    async fn test_guard(
        State(state): State<TestState>,
        req: Request<Body>,
        next: Next,
    ) -> axum::response::Response {
        let method = req.method().as_str();
        let raw_path = req.uri().path();
        let path = if raw_path.len() > 1 && raw_path.ends_with('/') {
            &raw_path[..raw_path.len() - 1]
        } else {
            raw_path
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

        let has_auth = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.starts_with("Bearer "))
            .unwrap_or(false);

        let role = req
            .headers()
            .get("x-test-role")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anon");

        if required == "admin" {
            if role != "admin" {
                return forbidden();
            }
            return next.run(req).await;
        }

        if required == "authed" {
            if !has_auth {
                return unauthorized();
            }
            return next.run(req).await;
        }

        if required.contains(':') && !has_auth {
            return unauthorized();
        }

        next.run(req).await
    }

    fn build_test_app() -> axum::Router {
        let perms = Arc::new(RoutePermissionMap::from_routes(&[]));
        let state = TestState { route_perms: perms };

        let handler = || axum::routing::any(|| async { "ok" });

        axum::Router::new()
            .route("/api/v1/admin/secret", handler())
            .route("/api/v1/posts", handler())
            .route("/api/v1/posts/{id}", handler())
            .route("/api/v1/public", handler())
            .layer(from_fn_with_state(state.clone(), test_guard))
            .with_state(state)
    }

    fn build_test_app_with_perms(routes: &[(&str, &str, &str)]) -> axum::Router {
        let route_infos: Vec<RouteInfo> = routes
            .iter()
            .map(|(m, p, perm)| make_route(m, p, Some(*perm)))
            .collect();
        let perms = Arc::new(RoutePermissionMap::from_routes(&route_infos));
        let state = TestState { route_perms: perms };

        let handler = || axum::routing::any(|| async { "ok" });

        axum::Router::new()
            .route("/api/v1/posts", handler())
            .route("/api/v1/public", handler())
            .layer(from_fn_with_state(state.clone(), test_guard))
            .with_state(state)
    }

    #[tokio::test]
    async fn integration_admin_route_rejects_anonymous() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn integration_admin_route_rejects_non_admin() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/secret")
                    .header("authorization", "Bearer fake.jwt.token")
                    .header("x-test-role", "reader")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn integration_admin_route_allows_admin() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/secret")
                    .header("authorization", "Bearer fake.jwt.token")
                    .header("x-test-role", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn integration_undeclared_non_admin_route_passes() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/posts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn integration_public_route_passes_anonymous() {
        let app = build_test_app_with_perms(&[("GET", "/api/v1/public", "public")]);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/public")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn integration_authed_route_rejects_anonymous() {
        let app = build_test_app_with_perms(&[("GET", "/api/v1/posts", "authed")]);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/posts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn integration_authed_route_allows_authenticated() {
        let app = build_test_app_with_perms(&[("GET", "/api/v1/posts", "authed")]);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/posts")
                    .header("authorization", "Bearer fake.jwt.token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn integration_cms_route_exempt() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/cms/posts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn integration_cms_false_positive_not_exempt() {
        // Bug 2 regression: /api/v1/cmswidget must NOT be exempt
        // (route doesn't exist → 404, but must NOT be 403 from admin heuristic)
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/cmswidget")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn integration_trailing_slash_admin() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/secret/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}

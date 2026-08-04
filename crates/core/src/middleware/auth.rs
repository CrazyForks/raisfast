//! Unified authentication extractor
//!
//! Resolves user identity and tenant from JWT / API Token + `X-Tenant-ID` header combined.
//! Never rejects; when not logged in, `user_id()` returns `None`.
//!
//! # Usage
//!
//! ```ignore
//! // Require authentication
//! async fn create(auth: AuthUser, ...) {
//!     let user_id = auth.ensure_authenticated()?;
//!     ...
//! }
//!
//! // Require admin
//! async fn cron_list(auth: AuthUser, ...) {
//!     auth.ensure_admin()?;
//!     ...
//! }
//!
//! // Public endpoint
//! async fn public_list(auth: AuthUser, ...) {
//!     // Don't call ensure_*, just use auth.tenant_id()
//! }
//! ```
//!
//! # Tenant resolution rules
//!
//! | Scenario | tenant_id | Description |
//! |---|---|---|
//! | Super admin + `X-Tenant-ID` | `Some(header)` | Super admin switches to specified tenant |
//! | Super admin + no Header | `None` | Super admin views all tenant data |
//! | Regular user | `Some(jwt_tenant_id)` | Ignores header, uses tenant from JWT |
//! | Not logged in + `X-Tenant-ID` | `Some(header)` | Public API specifies tenant |
//! | Not logged in + no Header | `Some("default")` | Fallback |

use crate::types::snowflake_id::SnowflakeId;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::models::user::UserRole;

/// Fine-grained token scope actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenAction {
    Read,
    Create,
    Update,
    Delete,
}

impl TokenAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Claims {
    pub user_id: SnowflakeId,
    pub roles: Vec<UserRole>,
    pub role_ids: Vec<i64>,
    pub tenant_id: String,
    pub scopes: Vec<String>,
}

impl Claims {
    pub(crate) fn has_role(&self, role: UserRole) -> bool {
        self.roles.contains(&role)
    }
}

#[derive(Debug, Clone)]
struct RequestIdentity {
    user_id: Option<i64>,
    roles: Vec<UserRole>,
    role_ids: Vec<i64>,
    tenant_id: Option<String>,
    is_super_admin: bool,
    token_present_but_invalid: bool,
    /// API-token scopes; empty = no restriction (JWT or no token).
    scopes: Vec<String>,
}

/// Unified identity extractor.
///
/// Never rejects — when not logged in, `user_id()` returns `None`.
/// Call `ensure_*` methods for role/authentication guards.
#[derive(Debug, Clone)]
pub struct AuthUser(RequestIdentity);

impl AuthUser {
    pub fn user_id(&self) -> Option<i64> {
        self.0.user_id
    }

    pub fn role(&self) -> &str {
        self.0
            .roles
            .first()
            .copied()
            .map(UserRole::as_str)
            .unwrap_or(UserRole::Reader.as_str())
    }

    /// All roles assigned to the current user.
    pub fn roles(&self) -> &[UserRole] {
        &self.0.roles
    }

    pub fn role_ids(&self) -> &[i64] {
        &self.0.role_ids
    }

    /// Whether the current user has a specific role.
    pub fn has_role(&self, role: UserRole) -> bool {
        self.0.roles.contains(&role)
    }

    pub fn tenant_id(&self) -> Option<&str> {
        self.0.tenant_id.as_deref()
    }

    pub fn is_authenticated(&self) -> bool {
        self.0.user_id.is_some()
    }

    pub fn is_admin(&self) -> bool {
        self.has_role(UserRole::Admin)
    }

    pub fn is_author(&self) -> bool {
        self.has_role(UserRole::Author) || self.has_role(UserRole::Admin)
    }

    pub fn is_super_admin(&self) -> bool {
        self.0.is_super_admin
    }

    /// API-token scopes (empty for JWT login = unrestricted).
    pub fn scopes(&self) -> &[String] {
        &self.0.scopes
    }

    /// Check whether the current token grants access to `resource:action`.
    ///
    /// Empty scopes (JWT login, or no token) → always allowed (role-based gate applies instead).
    /// Supported wildcard patterns:
    /// - `*` → all resources, all actions
    /// - `resource:*` → all actions on a specific resource
    /// - `*:read` → all resources, specific action
    /// - `resource:read` → exact match
    pub fn has_scope(&self, resource: &str, action: TokenAction) -> bool {
        if self.0.scopes.is_empty() {
            return true;
        }
        let act = action.as_str();
        self.0.scopes.iter().any(|s| {
            s == "*"
                || s == &format!("{resource}:*")
                || s == &format!("*:{act}")
                || s == &format!("{resource}:{act}")
        })
    }

    /// Guard: ensure the token scope grants `resource:action`.
    ///
    /// This is a scope-only check: it does not gate authentication. Anonymous
    /// requests (no token) have empty scopes and pass here; the actual access
    /// level (`public`/`authed`/`admin`) is enforced separately by the handler
    /// (e.g. `check_api_access` / `ensure_admin`).
    pub fn ensure_scope(&self, resource: &str, action: TokenAction) -> AppResult<()> {
        if self.0.token_present_but_invalid {
            return Err(AppError::Unauthorized);
        }
        if !self.has_scope(resource, action) {
            return Err(AppError::ForbiddenScope(format!(
                "{}:{}",
                resource,
                action.as_str()
            )));
        }
        Ok(())
    }

    pub fn ensure_authenticated(&self) -> AppResult<i64> {
        if self.0.token_present_but_invalid {
            return Err(AppError::Unauthorized);
        }
        self.0.user_id.ok_or(AppError::Unauthorized)
    }

    pub fn ensure_snowflake_user_id(&self) -> AppResult<crate::types::snowflake_id::SnowflakeId> {
        if self.0.token_present_but_invalid {
            return Err(AppError::Unauthorized);
        }
        self.0
            .user_id
            .map(crate::types::snowflake_id::SnowflakeId)
            .ok_or(AppError::Unauthorized)
    }

    pub fn ensure_admin(&self) -> AppResult<()> {
        if self.0.token_present_but_invalid {
            return Err(AppError::Unauthorized);
        }
        if self.is_authenticated() && self.is_admin() {
            Ok(())
        } else {
            Err(AppError::ForbiddenAdmin)
        }
    }

    pub fn ensure_author(&self) -> AppResult<()> {
        if self.0.token_present_but_invalid {
            return Err(AppError::Unauthorized);
        }
        if self.is_authenticated() && self.is_author() {
            Ok(())
        } else {
            Err(AppError::ForbiddenRbac("author role required".to_string()))
        }
    }

    pub fn from_parts(user_id: Option<i64>, role: UserRole, tenant_id: Option<String>) -> Self {
        AuthUser(RequestIdentity {
            user_id,
            roles: vec![role],
            role_ids: Vec::new(),
            tenant_id,
            is_super_admin: false,
            token_present_but_invalid: false,
            scopes: Vec::new(),
        })
    }
}

#[cfg(test)]
impl AuthUser {
    pub fn new_test(user_id: i64, role: UserRole, tenant_id: &str) -> Self {
        let uid = if user_id == 0 { None } else { Some(user_id) };
        AuthUser(RequestIdentity {
            user_id: uid,
            roles: vec![role],
            role_ids: Vec::new(),
            tenant_id: if tenant_id.is_empty() {
                None
            } else {
                Some(tenant_id.to_string())
            },
            is_super_admin: false,
            token_present_but_invalid: false,
            scopes: Vec::new(),
        })
    }

    pub fn new_test_super_admin(user_id: i64, tenant_id: &str) -> Self {
        let uid = if user_id == 0 { None } else { Some(user_id) };
        AuthUser(RequestIdentity {
            user_id: uid,
            roles: vec![UserRole::Admin],
            role_ids: Vec::new(),
            tenant_id: if tenant_id.is_empty() {
                None
            } else {
                Some(tenant_id.to_string())
            },
            is_super_admin: true,
            token_present_but_invalid: false,
            scopes: Vec::new(),
        })
    }

    pub fn new_test_with_scopes(
        user_id: i64,
        role: UserRole,
        tenant_id: &str,
        scopes: Vec<String>,
    ) -> Self {
        let mut auth = Self::new_test(user_id, role, tenant_id);
        auth.0.scopes = scopes;
        auth
    }
}

fn extract_header_tenant(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(crate::constants::HEADER_TENANT_ID)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(std::string::ToString::to_string)
}

fn extract_bearer_token(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(crate::constants::HEADER_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix(crate::constants::AUTH_BEARER_PREFIX))
}

/// Resolve a bearer token string to identity [`Claims`].
///
/// Handles both API tokens (`rf_*` prefix) and JWT tokens.
/// Returns `None` if the token is invalid or expired.
pub(crate) async fn resolve_bearer(token: &str, state: &AppState) -> Option<Claims> {
    if crate::services::api_token::is_api_token(token) {
        let (user_id, role_names, scopes, tenant_id) =
            crate::services::api_token::verify_api_token(&state.pool, &*state.cache, token)
                .await
                .ok()?;
        let roles: Vec<UserRole> = role_names.iter().filter_map(|r| r.parse().ok()).collect();
        let role_ids = resolve_role_ids_cached(&state.pool, &*state.cache, &role_names).await;
        let tenant_id = tenant_id.unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
        Some(Claims {
            user_id: SnowflakeId(user_id),
            roles,
            role_ids,
            tenant_id,
            scopes,
        })
    } else {
        let jwt_claims =
            crate::services::auth::verify_token(token, &state.jwt_decoding_key).ok()?;
        // role_names and role_ids are embedded in the JWT — zero DB/cache queries
        let roles: Vec<UserRole> = jwt_claims
            .role_names
            .iter()
            .filter_map(|r| r.parse().ok())
            .collect();
        Some(Claims {
            user_id: jwt_claims.sub.parse().ok()?,
            roles,
            role_ids: jwt_claims.role_ids,
            tenant_id: jwt_claims.tenant_id,
            scopes: Vec::new(),
        })
    }
}

/// Resolve role names to role IDs, with cache-aside (60s TTL).
///
/// Avoids hitting the DB on every request for the same set of role names.
async fn resolve_role_ids_cached(
    pool: &crate::db::Pool,
    cache: &dyn crate::cache::CacheStore,
    role_names: &[String],
) -> Vec<i64> {
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

    let mut ids = Vec::with_capacity(role_names.len());
    for name in role_names {
        let cache_key = format!("role_id:{name}");
        if let Some(cached) = cache.get(&cache_key).await
            && let Ok(id) = cached.parse::<i64>()
        {
            ids.push(id);
            continue;
        }
        if let Ok(Some(id)) = crate::models::rbac::find_role_id_by_name(pool, name).await {
            let _ = cache
                .set(&cache_key, &id.to_string(), Some(CACHE_TTL))
                .await;
            ids.push(id);
        }
    }
    ids
}

async fn extract_claims(parts: &Parts, state: &AppState) -> Option<Claims> {
    // Reuse identity resolved by permission_guard middleware if available
    if let Some(cached) = parts.extensions.get::<Claims>() {
        return Some(cached.clone());
    }
    let token = extract_bearer_token(parts)?;
    resolve_bearer(token, state).await
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let header_tenant = extract_header_tenant(parts);
        let has_token = extract_bearer_token(parts).is_some();
        let claims_fut = extract_claims(parts, state);

        async move {
            let claims = claims_fut.await;
            let no_tenant = !state.config.builtin_tenantable;
            let token_invalid = has_token && claims.is_none();

            let identity = match (claims, header_tenant) {
                (Some(c), Some(ht)) if c.has_role(UserRole::Admin) => RequestIdentity {
                    user_id: Some(*c.user_id),
                    roles: c.roles,
                    role_ids: c.role_ids,
                    tenant_id: if no_tenant { None } else { Some(ht) },
                    is_super_admin: true,
                    token_present_but_invalid: false,
                    scopes: c.scopes,
                },
                (Some(c), None) if c.has_role(UserRole::Admin) => RequestIdentity {
                    user_id: Some(*c.user_id),
                    roles: c.roles,
                    role_ids: c.role_ids,
                    tenant_id: None,
                    is_super_admin: true,
                    token_present_but_invalid: false,
                    scopes: c.scopes,
                },
                (Some(c), _) => RequestIdentity {
                    user_id: Some(*c.user_id),
                    roles: c.roles,
                    role_ids: c.role_ids,
                    tenant_id: if no_tenant { None } else { Some(c.tenant_id) },
                    is_super_admin: false,
                    token_present_but_invalid: false,
                    scopes: c.scopes,
                },
                (None, Some(ht)) => RequestIdentity {
                    user_id: None,
                    roles: vec![UserRole::Reader],
                    role_ids: Vec::new(),
                    tenant_id: if no_tenant { None } else { Some(ht) },
                    is_super_admin: false,
                    token_present_but_invalid: token_invalid,
                    scopes: Vec::new(),
                },
                (None, None) => RequestIdentity {
                    user_id: None,
                    roles: vec![UserRole::Reader],
                    role_ids: Vec::new(),
                    tenant_id: if no_tenant {
                        None
                    } else {
                        Some(crate::constants::DEFAULT_TENANT.to_string())
                    },
                    is_super_admin: false,
                    token_present_but_invalid: token_invalid,
                    scopes: Vec::new(),
                },
            };

            Ok(AuthUser(identity))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::app_error::AppError;

    #[test]
    fn from_parts_all_fields_accessors() {
        let auth = AuthUser::from_parts(Some(42), UserRole::Author, Some("tenant-1".to_string()));
        assert_eq!(auth.user_id(), Some(42));
        assert_eq!(auth.role(), "author");
        assert_eq!(auth.tenant_id(), Some("tenant-1"));
        assert!(auth.is_authenticated());
    }

    #[test]
    fn from_parts_no_user_id_not_authenticated() {
        let auth = AuthUser::from_parts(None, UserRole::Reader, Some("t1".to_string()));
        assert!(!auth.is_authenticated());
        assert!(auth.user_id().is_none());
        let err = auth.ensure_authenticated().unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[test]
    fn admin_role_passes_admin_checks() {
        let auth = AuthUser::from_parts(Some(1), UserRole::Admin, Some("t1".to_string()));
        assert!(auth.is_admin());
        assert!(auth.ensure_admin().is_ok());
        assert!(auth.is_author());
        assert!(auth.ensure_author().is_ok());
    }

    #[test]
    fn reader_role_denied_admin_and_author() {
        let auth = AuthUser::from_parts(Some(1), UserRole::Reader, Some("t1".to_string()));
        assert!(!auth.is_admin());
        assert!(matches!(
            auth.ensure_admin().unwrap_err(),
            AppError::ForbiddenAdmin
        ));
        assert!(!auth.is_author());
        assert!(matches!(
            auth.ensure_author().unwrap_err(),
            AppError::ForbiddenRbac(_)
        ));
    }

    #[test]
    fn author_role_passes_author_checks() {
        let auth = AuthUser::from_parts(Some(1), UserRole::Author, Some("t1".to_string()));
        assert!(auth.is_author());
        assert!(auth.ensure_author().is_ok());
        assert!(!auth.is_admin());
        assert!(matches!(
            auth.ensure_admin().unwrap_err(),
            AppError::ForbiddenAdmin
        ));
    }

    #[test]
    fn super_admin_flag_true() {
        let auth = AuthUser::new_test_super_admin(1, "t1");
        assert!(auth.is_super_admin());
        assert!(auth.is_admin());
        assert!(auth.is_authenticated());
    }

    #[test]
    fn from_parts_super_admin_flag_false() {
        let auth = AuthUser::from_parts(Some(1), UserRole::Admin, Some("t1".to_string()));
        assert!(!auth.is_super_admin());
    }

    #[test]
    fn tenant_id_some() {
        let auth = AuthUser::from_parts(Some(1), UserRole::Reader, Some("my-tenant".to_string()));
        assert_eq!(auth.tenant_id(), Some("my-tenant"));
    }

    #[test]
    fn tenant_id_none() {
        let auth = AuthUser::from_parts(Some(1), UserRole::Reader, None);
        assert!(auth.tenant_id().is_none());
    }

    #[test]
    fn unauthenticated_ensure_admin_and_author_forbidden() {
        let auth = AuthUser::from_parts(None, UserRole::Reader, None);
        assert!(matches!(
            auth.ensure_admin().unwrap_err(),
            AppError::ForbiddenAdmin
        ));
        assert!(matches!(
            auth.ensure_author().unwrap_err(),
            AppError::ForbiddenRbac(_)
        ));
    }

    #[test]
    fn new_test_with_zero_id_is_anonymous() {
        let auth = AuthUser::new_test(0, UserRole::Reader, "");
        assert!(!auth.is_authenticated());
        assert!(auth.user_id().is_none());
        assert!(auth.tenant_id().is_none());
    }

    #[test]
    fn editor_role_not_admin_not_author() {
        let auth = AuthUser::from_parts(Some(1), UserRole::Editor, Some("t1".to_string()));
        assert!(!auth.is_admin());
        assert!(!auth.is_author());
    }
}

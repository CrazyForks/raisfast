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

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::models::user::UserRole;

struct Claims {
    user_id: String,
    user_int_id: i64,
    role: UserRole,
    tenant_id: String,
}

#[derive(Debug, Clone)]
struct RequestIdentity {
    user_id: Option<String>,
    user_int_id: Option<i64>,
    role: UserRole,
    tenant_id: Option<String>,
    is_super_admin: bool,
}

/// Unified identity extractor.
///
/// Never rejects — when not logged in, `user_id()` returns `None`.
/// Call `ensure_*` methods for role/authentication guards.
#[derive(Debug, Clone)]
pub struct AuthUser(RequestIdentity);

impl AuthUser {
    pub fn user_id(&self) -> Option<&str> {
        self.0.user_id.as_deref()
    }

    pub fn user_int_id(&self) -> Option<i64> {
        self.0.user_int_id
    }

    pub fn role(&self) -> &str {
        self.0.role.as_str()
    }

    pub fn tenant_id(&self) -> Option<&str> {
        self.0.tenant_id.as_deref()
    }

    pub fn is_authenticated(&self) -> bool {
        self.0.user_id.is_some()
    }

    pub fn is_admin(&self) -> bool {
        self.0.role == UserRole::Admin
    }

    pub fn is_author(&self) -> bool {
        self.0.role == UserRole::Author || self.0.role == UserRole::Admin
    }

    pub fn is_super_admin(&self) -> bool {
        self.0.is_super_admin
    }

    pub fn ensure_authenticated(&self) -> AppResult<&str> {
        self.0.user_id.as_deref().ok_or(AppError::Unauthorized)
    }

    pub fn ensure_admin(&self) -> AppResult<()> {
        if self.is_authenticated() && self.is_admin() {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    pub fn ensure_author(&self) -> AppResult<()> {
        if self.is_authenticated() && self.is_author() {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    pub fn from_parts(
        user_id: Option<String>,
        user_int_id: Option<i64>,
        role: UserRole,
        tenant_id: Option<String>,
    ) -> Self {
        AuthUser(RequestIdentity {
            user_id,
            user_int_id,
            role,
            tenant_id,
            is_super_admin: false,
        })
    }
}

#[cfg(test)]
impl AuthUser {
    pub fn new_test(user_id: &str, role: UserRole, tenant_id: &str) -> Self {
        AuthUser(RequestIdentity {
            user_id: if user_id.is_empty() {
                None
            } else {
                Some(user_id.to_string())
            },
            user_int_id: None,
            role,
            tenant_id: if tenant_id.is_empty() {
                None
            } else {
                Some(tenant_id.to_string())
            },
            is_super_admin: false,
        })
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

async fn extract_claims(parts: &Parts, state: &AppState) -> Option<Claims> {
    let token = extract_bearer_token(parts)?;

    if crate::services::api_token::is_api_token(token) {
        let (user_id, user_int_id, role, tenant_id) =
            crate::services::api_token::verify_api_token(&state.pool, &*state.cache, token)
                .await
                .ok()?;
        let role: UserRole = role.parse().ok()?;
        Some(Claims {
            user_id,
            user_int_id,
            role,
            tenant_id: tenant_id.unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string()),
        })
    } else {
        let claims = crate::services::auth::verify_token(token, &state.jwt_decoding_key).ok()?;
        Some(Claims {
            user_id: claims.sub,
            user_int_id: claims.iid,
            role: claims.role,
            tenant_id: claims.tenant_id,
        })
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let header_tenant = extract_header_tenant(parts);
        let claims_fut = extract_claims(parts, state);

        async move {
            let claims = claims_fut.await;
            let no_tenant = !state.config.builtin_tenantable;

            let identity = match (claims, header_tenant) {
                (Some(c), Some(ht)) if c.role == UserRole::Admin => RequestIdentity {
                    user_id: Some(c.user_id),
                    user_int_id: Some(c.user_int_id),
                    role: c.role,
                    tenant_id: if no_tenant { None } else { Some(ht) },
                    is_super_admin: true,
                },
                (Some(c), None) if c.role == UserRole::Admin => RequestIdentity {
                    user_id: Some(c.user_id),
                    user_int_id: Some(c.user_int_id),
                    role: c.role,
                    tenant_id: None,
                    is_super_admin: true,
                },
                (Some(c), _) => RequestIdentity {
                    user_id: Some(c.user_id),
                    user_int_id: Some(c.user_int_id),
                    role: c.role,
                    tenant_id: if no_tenant { None } else { Some(c.tenant_id) },
                    is_super_admin: false,
                },
                (None, Some(ht)) => RequestIdentity {
                    user_id: None,
                    user_int_id: None,
                    role: UserRole::Reader,
                    tenant_id: if no_tenant { None } else { Some(ht) },
                    is_super_admin: false,
                },
                (None, None) => RequestIdentity {
                    user_id: None,
                    user_int_id: None,
                    role: UserRole::Reader,
                    tenant_id: if no_tenant {
                        None
                    } else {
                        Some(crate::constants::DEFAULT_TENANT.to_string())
                    },
                    is_super_admin: false,
                },
            };

            Ok(AuthUser(identity))
        }
    }
}

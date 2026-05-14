//! RBAC permission guard middleware
//!
//! Replaces hardcoded `role == "admin"` checks with fine-grained authorization
//! via the permissions table. Retains existing `AuthUser` / `AdminUser` / `AuthorUser`
//! extractors as convenience entry points, and adds `PermissionGuard` for scenarios
//! requiring dynamic permission checks.

use std::collections::HashMap;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde_json::Value;

use crate::AppState;
use crate::errors::app_error::AppError;
use crate::services::rbac::RbacService;
use crate::utils::auth::extract_claims;

/// RBAC permission guard extractor
///
/// Extracts user info from JWT, then checks whether the user's role has permission
/// to perform the specified action.
///
/// # Usage
///
/// ```ignore
/// async fn create_post(
///     guard: PermissionGuard,
///     State(state): State<AppState>,
/// ) -> ... {
///     // guard has passed permission check
///     guard.user_id  // current user ID
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PermissionGuard {
    pub user_id: String,
    pub role: crate::models::user::UserRole,
    pub tenant_id: String,
}

impl PermissionGuard {
    /// Public method for checking permissions, usable within handlers
    pub async fn check(
        &self,
        rbac: &RbacService,
        action: &str,
        subject: &str,
    ) -> Result<(), AppError> {
        let role_id = rbac
            .get_role_id_by_name(self.role.as_str())
            .await?
            .map(|id| id.to_string())
            .unwrap_or_else(|| self.role.as_str().to_string());

        rbac.check_permission(&role_id, action, subject, None).await
    }

    pub async fn check_with_context(
        &self,
        rbac: &RbacService,
        action: &str,
        subject: &str,
        context: &HashMap<String, Value>,
    ) -> Result<(), AppError> {
        let role_id = rbac
            .get_role_id_by_name(self.role.as_str())
            .await?
            .map(|id| id.to_string())
            .unwrap_or_else(|| self.role.as_str().to_string());

        rbac.check_permission(&role_id, action, subject, Some(context))
            .await
    }
}

/// When `PermissionGuard` is used as an extractor, it only performs JWT authentication
/// (does not automatically check permissions). Permission checking is done by calling
/// `guard.check()` within the handler.
impl FromRequestParts<AppState> for PermissionGuard {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_claims(parts, state);
        async move {
            let claims = result?;
            Ok(PermissionGuard {
                user_id: claims.sub,
                role: claims.role,
                tenant_id: claims.tenant_id,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserRole;

    #[test]
    fn construction_fields_match() {
        let guard = PermissionGuard {
            user_id: "user-123".to_string(),
            role: UserRole::Author,
            tenant_id: "tenant-1".to_string(),
        };
        assert_eq!(guard.user_id, "user-123");
        assert_eq!(guard.role, UserRole::Author);
        assert_eq!(guard.tenant_id, "tenant-1");
    }

    #[test]
    fn admin_role_field() {
        let guard = PermissionGuard {
            user_id: "admin-1".to_string(),
            role: UserRole::Admin,
            tenant_id: "t1".to_string(),
        };
        assert_eq!(guard.role, UserRole::Admin);
        assert_eq!(guard.role.as_str(), "admin");
    }

    #[test]
    fn clone_and_debug() {
        let guard = PermissionGuard {
            user_id: "u1".to_string(),
            role: UserRole::Reader,
            tenant_id: "t1".to_string(),
        };
        let cloned = guard.clone();
        assert_eq!(cloned.user_id, guard.user_id);
        assert_eq!(cloned.role.as_str(), guard.role.as_str());
        assert_eq!(cloned.tenant_id, guard.tenant_id);
        let debug_str = format!("{guard:?}");
        assert!(debug_str.contains("u1"));
    }
}

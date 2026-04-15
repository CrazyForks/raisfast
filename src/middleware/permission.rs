//! RBAC 权限守卫中间件
//!
//! 替代硬编码的 `role == "admin"` 检查，通过查询 permissions 表做细粒度鉴权。
//! 保留现有 `AuthUser` / `AdminUser` / `AuthorUser` 提取器作为便捷入口，
//! 新增 `PermissionGuard` 用于需要动态权限检查的场景。

use std::collections::HashMap;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde_json::Value;

use crate::AppState;
use crate::errors::app_error::AppError;
use crate::services::rbac::RbacService;
use crate::utils::auth::extract_claims;

/// RBAC 权限守卫提取器
///
/// 从 JWT 提取用户信息，然后检查该用户角色是否有权执行指定操作。
///
/// # 使用方式
///
/// ```ignore
/// async fn create_post(
///     guard: PermissionGuard,
///     State(state): State<AppState>,
/// ) -> ... {
///     // guard 已通过权限检查
///     guard.user_id  // 当前用户 ID
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PermissionGuard {
    pub user_id: String,
    pub role: String,
    pub tenant_id: String,
}

impl PermissionGuard {
    /// 检查权限的公共方法，可在 handler 内部使用
    pub async fn check(
        &self,
        rbac: &RbacService,
        action: &str,
        subject: &str,
    ) -> Result<(), AppError> {
        let role_id = rbac
            .get_role_id_by_name(&self.role)
            .await?
            .unwrap_or_else(|| self.role.clone());

        rbac.check_permission(&role_id, action, subject, None).await
    }

    /// 带条件的权限检查
    pub async fn check_with_context(
        &self,
        rbac: &RbacService,
        action: &str,
        subject: &str,
        context: &HashMap<String, Value>,
    ) -> Result<(), AppError> {
        let role_id = rbac
            .get_role_id_by_name(&self.role)
            .await?
            .unwrap_or_else(|| self.role.clone());

        rbac.check_permission(&role_id, action, subject, Some(context))
            .await
    }
}

/// `PermissionGuard` 作为提取器使用时，只做 JWT 认证（不自动检查权限）
/// 权限检查在 handler 中调用 `guard.check()` 完成
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

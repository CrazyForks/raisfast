//! 认证工具函数

use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthIdentity;
use crate::services::auth::Claims;

/// 从请求头中提取并验证 JWT claims。
///
/// 读取 `Authorization: Bearer <token>` 头，验证签名和有效期。
/// 各认证提取器共用此函数，避免重复代码。
pub fn extract_claims(parts: &mut Parts, state: &AppState) -> Result<Claims, AppError> {
    let secret = &state.config.jwt_secret;
    let auth_header = parts
        .headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    crate::services::auth::verify_token(token, secret)
}

/// 尝试从请求头中提取 JWT claims，无 token 时返回 None（不报错）。
pub fn extract_optional_identity(parts: &mut Parts, state: &AppState) -> Option<AuthIdentity> {
    extract_claims(parts, state)
        .ok()
        .map(|claims| AuthIdentity {
            user_id: claims.sub,
            role: claims.role,
            tenant_id: claims.tenant_id,
        })
}

/// 校验当前用户是管理员或资源所有者，否则返回 `Forbidden`。
///
/// # 参数
///
/// - `role` — 当前用户角色
/// - `user_id` — 当前用户 ID
/// - `owner_id` — 资源所有者 ID
pub fn require_owner_or_admin(role: &str, user_id: &str, owner_id: &str) -> AppResult<()> {
    if role != "admin" && owner_id != user_id {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

/// 与 [`require_owner_or_admin`] 相同逻辑，但 `owner_id` 为 `Option`（如访客评论）。
pub fn require_owner_or_admin_opt(
    role: &str,
    user_id: &str,
    owner_id: Option<&str>,
) -> AppResult<()> {
    if role != "admin" && owner_id != Some(user_id) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

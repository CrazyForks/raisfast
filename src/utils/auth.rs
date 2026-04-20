//! 认证工具函数

use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthIdentity;
use crate::services::auth::Claims;

/// 从请求头中提取并验证身份（JWT 或 API Token）。
///
/// 读取 `Authorization: Bearer <token>` 头：
/// - 若 token 以 `rblog_` 开头，走 API Token 验证路径
/// - 否则走 JWT 验证路径
pub fn extract_claims(parts: &mut Parts, state: &AppState) -> Result<Claims, AppError> {
    let auth_header = parts
        .headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    crate::services::auth::verify_token(token, &state.jwt_decoding_key)
}

/// 尝试从请求头中提取身份（JWT 或 API Token），无 token 时返回 None。
pub fn extract_optional_identity(parts: &mut Parts, state: &AppState) -> Option<AuthIdentity> {
    extract_claims(parts, state)
        .ok()
        .map(|claims| AuthIdentity {
            user_id: claims.sub,
            role: claims.role,
            tenant_id: claims.tenant_id,
        })
}

/// 从请求头中提取并验证身份，支持 JWT 和 API Token 两种方式。
///
/// 返回 `AuthIdentity`，供 extractor 使用。
pub async fn extract_identity(parts: &mut Parts, state: &AppState) -> AppResult<AuthIdentity> {
    let auth_header = parts
        .headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    if crate::services::api_token::is_api_token(token) {
        let (user_id, role, tenant_id) =
            crate::services::api_token::verify_api_token(&state.pool, token).await?;
        return Ok(AuthIdentity {
            user_id,
            role,
            tenant_id,
        });
    }

    let claims = crate::services::auth::verify_token(token, &state.jwt_decoding_key)?;
    Ok(AuthIdentity {
        user_id: claims.sub,
        role: claims.role,
        tenant_id: claims.tenant_id,
    })
}

/// 异步版本的 extract_optional_identity，支持 API Token。
pub async fn extract_optional_identity_async(
    parts: &mut Parts,
    state: &AppState,
) -> Option<AuthIdentity> {
    extract_identity(parts, state).await.ok()
}

/// 校验当前用户是管理员或资源所有者，否则返回 `Forbidden`。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_owner_or_admin_allows_owner() {
        assert!(require_owner_or_admin("author", "u1", "u1").is_ok());
    }

    #[test]
    fn require_owner_or_admin_allows_admin() {
        assert!(require_owner_or_admin("admin", "u1", "u2").is_ok());
    }

    #[test]
    fn require_owner_or_admin_rejects_other() {
        assert!(require_owner_or_admin("author", "u1", "u2").is_err());
    }

    #[test]
    fn require_owner_or_admin_opt_allows_owner() {
        assert!(require_owner_or_admin_opt("author", "u1", Some("u1")).is_ok());
    }

    #[test]
    fn require_owner_or_admin_opt_allows_admin_with_none() {
        assert!(require_owner_or_admin_opt("admin", "u1", None).is_ok());
    }

    #[test]
    fn require_owner_or_admin_opt_rejects_reader() {
        assert!(require_owner_or_admin_opt("reader", "u1", Some("u2")).is_err());
    }
}

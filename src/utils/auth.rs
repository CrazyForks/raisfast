//! 认证工具函数

use axum::http::request::Parts;

use crate::errors::app_error::{AppError, AppResult};
use crate::services::auth::Claims;
use crate::AppState;

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

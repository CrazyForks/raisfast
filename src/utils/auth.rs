//! Authentication utility functions

use axum::http::request::Parts;

use crate::errors::app_error::{AppError, AppResult};
use crate::services::auth::Claims;
use crate::AppState;

pub fn extract_claims(parts: &mut Parts, state: &AppState) -> Result<Claims, AppError> {
    let auth_header = parts
        .headers
        .get(crate::constants::HEADER_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix(crate::constants::AUTH_BEARER_PREFIX)
        .ok_or(AppError::Unauthorized)?;

    crate::services::auth::verify_token(token, &state.jwt_decoding_key)
}

/// Verifies that the current user is an admin or the resource owner; otherwise returns `Forbidden`.
pub fn require_owner_or_admin(role: &str, user_id: i64, owner_id: i64) -> AppResult<()> {
    if role != "admin" && owner_id != user_id {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

/// Same logic as [`require_owner_or_admin`], but `owner_id` is `Option` (e.g. guest comments).
pub fn require_owner_or_admin_opt(
    role: &str,
    user_id: i64,
    owner_id: Option<i64>,
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
        assert!(require_owner_or_admin("author", 1, 1).is_ok());
    }

    #[test]
    fn require_owner_or_admin_allows_admin() {
        assert!(require_owner_or_admin("admin", 1, 2).is_ok());
    }

    #[test]
    fn require_owner_or_admin_rejects_other() {
        assert!(require_owner_or_admin("author", 1, 2).is_err());
    }

    #[test]
    fn require_owner_or_admin_opt_allows_owner() {
        assert!(require_owner_or_admin_opt("author", 1, Some(1)).is_ok());
    }

    #[test]
    fn require_owner_or_admin_opt_allows_admin_with_none() {
        assert!(require_owner_or_admin_opt("admin", 1, None).is_ok());
    }

    #[test]
    fn require_owner_or_admin_opt_rejects_reader() {
        assert!(require_owner_or_admin_opt("reader", 1, Some(2)).is_err());
    }
}

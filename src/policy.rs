//! Resource-level authorization — ownership checks.
//!
//! Complements the global [`permission_guard`](crate::middleware::permission_guard)
//! middleware with per-resource ownership rules. The middleware checks
//! "can *role* do *action* on *subject*"; these functions check
//! "can *this user* touch *this specific resource*".
//!
//! Every check verifies tenant isolation first, then ownership.

use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;

/// Verify the user and resource belong to the same tenant.
///
/// `None` user tenant = system-level (super admin), bypasses tenant restriction.
fn check_tenant(user_tenant: Option<&str>, resource_tenant: Option<&str>) -> bool {
    match user_tenant {
        None => true,
        Some(u) => resource_tenant.is_some_and(|r| r == u),
    }
}

/// Check tenant isolation + ownership (non-optional owner).
///
/// # Errors
///
/// - [`AppError::Unauthorized`] if the user is not authenticated.
/// - [`AppError::Forbidden`] if tenant mismatch or the user is neither owner nor admin.
pub fn check_owner(
    user: &AuthUser,
    created_by: SnowflakeId,
    resource_tenant: Option<&str>,
) -> AppResult<()> {
    if !check_tenant(user.tenant_id(), resource_tenant) {
        return Err(AppError::Forbidden);
    }
    let uid = user.user_id().ok_or(AppError::Unauthorized)?;
    if user.is_admin() || SnowflakeId(uid) == created_by {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// Check tenant isolation + ownership (optional owner).
///
/// When `created_by` is `None` (e.g. guest-created resources), only admins pass.
///
/// # Errors
///
/// - [`AppError::Unauthorized`] if the user is not authenticated.
/// - [`AppError::Forbidden`] if tenant mismatch or the user is neither owner nor admin.
pub fn check_owner_opt(
    user: &AuthUser,
    created_by: Option<SnowflakeId>,
    resource_tenant: Option<&str>,
) -> AppResult<()> {
    if !check_tenant(user.tenant_id(), resource_tenant) {
        return Err(AppError::Forbidden);
    }
    let uid = user.user_id().ok_or(AppError::Unauthorized)?;
    if user.is_admin() || created_by == Some(SnowflakeId(uid)) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserRole;

    fn admin() -> AuthUser {
        AuthUser::from_parts(Some(1), UserRole::Admin, Some("t1".into()))
    }

    fn admin_tenant(t: &str) -> AuthUser {
        AuthUser::from_parts(Some(1), UserRole::Admin, Some(t.into()))
    }

    fn author(uid: i64) -> AuthUser {
        AuthUser::from_parts(Some(uid), UserRole::Author, Some("t1".into()))
    }

    fn author_tenant(uid: i64, t: &str) -> AuthUser {
        AuthUser::from_parts(Some(uid), UserRole::Author, Some(t.into()))
    }

    fn anon() -> AuthUser {
        AuthUser::from_parts(None, UserRole::Reader, Some("t1".into()))
    }

    // ── check_owner ──

    #[test]
    fn owner_allows_owner() {
        assert!(check_owner(&author(10), SnowflakeId(10), Some("t1")).is_ok());
    }

    #[test]
    fn owner_allows_admin() {
        assert!(check_owner(&admin(), SnowflakeId(99), Some("t1")).is_ok());
    }

    #[test]
    fn owner_rejects_other() {
        assert!(check_owner(&author(1), SnowflakeId(2), Some("t1")).is_err());
    }

    #[test]
    fn owner_rejects_anon() {
        assert!(check_owner(&anon(), SnowflakeId(1), Some("t1")).is_err());
    }

    // ── check_owner_opt ──

    #[test]
    fn opt_allows_owner() {
        assert!(check_owner_opt(&author(5), Some(SnowflakeId(5)), Some("t1")).is_ok());
    }

    #[test]
    fn opt_allows_admin_none() {
        assert!(check_owner_opt(&admin(), None, Some("t1")).is_ok());
    }

    #[test]
    fn opt_rejects_other() {
        assert!(check_owner_opt(&author(1), Some(SnowflakeId(2)), Some("t1")).is_err());
    }

    #[test]
    fn opt_rejects_anon() {
        assert!(check_owner_opt(&anon(), Some(SnowflakeId(1)), Some("t1")).is_err());
    }

    // ── tenant isolation ──

    #[test]
    fn owner_rejects_tenant_mismatch() {
        let user = author_tenant(10, "tenant_a");
        assert!(check_owner(&user, SnowflakeId(10), Some("tenant_b")).is_err());
    }

    #[test]
    fn opt_rejects_tenant_mismatch() {
        let user = author_tenant(5, "tenant_a");
        assert!(check_owner_opt(&user, Some(SnowflakeId(5)), Some("tenant_b")).is_err());
    }

    #[test]
    fn admin_rejects_tenant_mismatch() {
        let admin = admin_tenant("tenant_a");
        assert!(check_owner(&admin, SnowflakeId(99), Some("tenant_b")).is_err());
    }

    #[test]
    fn both_none_tenant_passes() {
        let user = AuthUser::from_parts(Some(1), UserRole::Admin, None);
        assert!(check_owner(&user, SnowflakeId(1), None).is_ok());
    }

    #[test]
    fn none_user_tenant_bypasses() {
        // System-level user (None tenant) can access any tenant's resource
        let user = AuthUser::from_parts(Some(1), UserRole::Author, None);
        assert!(check_owner(&user, SnowflakeId(1), Some("any_tenant")).is_ok());
    }

    #[test]
    fn some_user_none_resource_rejects() {
        let user = AuthUser::from_parts(Some(1), UserRole::Author, Some("t1".to_string()));
        assert!(check_owner(&user, SnowflakeId(1), None).is_err());
    }
}

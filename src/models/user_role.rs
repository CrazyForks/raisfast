//! User-role assignment model (many-to-many).

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::{DbDriver, Driver, Pool};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// Row model for the `user_roles` table.
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct UserRoleAssignment {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub user_id: SnowflakeId,
    pub role_id: SnowflakeId,
    pub created_at: Timestamp,
}

/// Fetch all role assignments for a user.
pub async fn find_by_user_id(
    pool: &Pool,
    user_id: SnowflakeId,
) -> AppResult<Vec<UserRoleAssignment>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "user_roles",
        UserRoleAssignment,
        where: ("user_id", user_id.0),
        order_by: "created_at"
    )?)
}

/// Fetch all role names for a user via `user_roles` → `roles` join.
pub async fn find_role_names_by_user_id(
    pool: &Pool,
    user_id: SnowflakeId,
) -> AppResult<Vec<String>> {
    let sql = format!(
        "SELECT r.name FROM user_roles ur \
         JOIN roles r ON r.id = ur.role_id \
         WHERE ur.user_id = {}",
        Driver::ph(1)
    );
    Ok(raisfast_derive::crud_scalar!(
        pool,
        String,
        &sql,
        [user_id.0],
        fetch_all
    )?)
}

/// Assign a role to a user (idempotent via UNIQUE constraint).
pub async fn assign_role(
    pool: &Pool,
    user_id: SnowflakeId,
    role_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    let id = crate::utils::id::new_snowflake_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(pool, "user_roles", [
        "id" => id,
        "tenant_id" => tenant_id,
        "user_id" => user_id.0,
        "role_id" => role_id.0,
        "created_at" => now,
    ])?;
    Ok(())
}

/// Look up a role by name and assign it to the user.
///
/// Silently does nothing if the role name is not found in the `roles` table.
pub async fn assign_role_by_name(
    pool: &Pool,
    user_id: SnowflakeId,
    role_name: &str,
    tenant_id: &str,
) -> AppResult<()> {
    if let Some(rid) = crate::models::rbac::find_role_id_by_name(pool, role_name).await? {
        assign_role(pool, user_id, SnowflakeId(rid), tenant_id).await?;
    }
    Ok(())
}

/// Resolve a list of [`UserRole`](crate::models::user::UserRole) values into role IDs,
/// skipping any names that are not present in the `roles` table.
pub async fn resolve_role_ids(
    pool: &Pool,
    roles: &[crate::models::user::UserRole],
) -> AppResult<Vec<SnowflakeId>> {
    let mut ids = Vec::with_capacity(roles.len());
    for r in roles {
        if let Some(rid) = crate::models::rbac::find_role_id_by_name(pool, r.as_str()).await? {
            ids.push(SnowflakeId(rid));
        }
    }
    Ok(ids)
}

/// Remove a role from a user.
pub async fn revoke_role(pool: &Pool, user_id: SnowflakeId, role_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "user_roles",
        where: AND(("user_id", user_id.0), ("role_id", role_id.0)))?;
    Ok(())
}

/// Replace all roles for a user (delete + insert, within a transaction).
pub async fn set_roles(
    pool: &Pool,
    user_id: SnowflakeId,
    role_ids: &[SnowflakeId],
    tenant_id: &str,
) -> AppResult<()> {
    crate::in_transaction!(pool, tx, {
        raisfast_derive::crud_delete!(&mut *tx, "user_roles", where: ("user_id", user_id.0))?;
        let now = crate::utils::tz::now_utc();
        for &role_id in role_ids {
            let id = crate::utils::id::new_snowflake_id();
            raisfast_derive::crud_insert!(&mut *tx, "user_roles", [
                "id" => id,
                "tenant_id" => tenant_id,
                "user_id" => user_id.0,
                "role_id" => role_id.0,
                "created_at" => now,
            ])?;
        }
        Ok(())
    })
}

//! RBAC model and database queries
//!
//! Defines data structures for the `roles` / `permissions` tables and all CRUD operations.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::commands::CreatePermissionCmd;
use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

/// Row model for the roles table
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Role {
    pub id: i64,
    pub document_id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Row model for the permissions table
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Permission {
    pub id: i64,
    pub document_id: String,
    pub role_id: i64,
    pub action: String,
    pub subject: String,
    pub fields: Option<String>,
    pub conditions: Option<String>,
    pub created_at: Timestamp,
}

/// List all roles
pub async fn list_roles(pool: &crate::db::Pool) -> AppResult<Vec<Role>> {
    raisfast_derive::check_schema!(
        "roles",
        "id",
        "document_id",
        "name",
        "description",
        "is_system",
        "created_at",
        "updated_at"
    );
    let roles = raisfast_derive::crud_list!(pool, "roles", Role, order_by: "name")?;
    Ok(roles)
}

/// Find role by document_id
pub async fn find_role_by_id(pool: &crate::db::Pool, document_id: &str) -> AppResult<Option<Role>> {
    let role = raisfast_derive::crud_find!(pool, "roles", Role, "document_id" => document_id)?;
    Ok(role)
}

/// Find role ID by role name (returns integer PK)
pub async fn find_role_id_by_name(pool: &crate::db::Pool, name: &str) -> AppResult<Option<i64>> {
    let sql = format!("SELECT id FROM roles WHERE name = {}", ph(1));
    Ok(raisfast_derive::crud_scalar!(
        pool,
        i64,
        &sql,
        [name],
        fetch_optional
    )?)
}

/// Create role
pub async fn create_role(
    pool: &crate::db::Pool,
    document_id: &str,
    name: &str,
    description: Option<&str>,
) -> AppResult<Role> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "roles",
        [
            "document_id" => document_id,
            "name" => name,
            "description" => description,
            "is_system" => 0_i64,
            "created_at" => now,
            "updated_at" => now
        ]
    )
    .map_err(|e| AppError::Conflict(format!("create role failed: {e}")))?;

    find_role_by_id(pool, document_id)
        .await?
        .ok_or_else(|| AppError::not_found("role"))
}

/// Update role (dynamic SET clause)
pub async fn update_role(
    pool: &crate::db::Pool,
    document_id: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> AppResult<Role> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool, "roles",
        bind: ["updated_at" => now],
        optional: ["name" => name, "description" => description],
        where: "document_id" => document_id
    )?;

    find_role_by_id(pool, document_id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("role/{document_id}")))
}

/// Delete role
pub async fn delete_role(pool: &crate::db::Pool, document_id: &str) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "roles", "document_id" => document_id)?;
    Ok(())
}

/// List all permissions for a role
pub async fn find_permissions_by_role_id(
    pool: &crate::db::Pool,
    role_id: i64,
) -> AppResult<Vec<Permission>> {
    raisfast_derive::check_schema!(
        "permissions",
        "id",
        "document_id",
        "role_id",
        "action",
        "subject",
        "fields",
        "conditions",
        "created_at"
    );
    let perms = raisfast_derive::crud_find_all!(pool, "permissions", Permission, "role_id" => role_id, order_by: "action")?;
    Ok(perms)
}

/// Delete all permissions for a role
pub async fn delete_permissions_by_role_id(pool: &crate::db::Pool, role_id: i64) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "permissions", "role_id" => role_id)?;
    Ok(())
}

/// Insert a single permission
pub async fn insert_permission(pool: &crate::db::Pool, cmd: &CreatePermissionCmd) -> AppResult<()> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    raisfast_derive::crud_insert!(pool, "permissions", [
        "document_id" => &document_id,
        "role_id" => cmd.role_id,
        "action" => &cmd.action,
        "subject" => &cmd.subject,
        "fields" => cmd.fields.clone(),
        "conditions" => cmd.conditions.clone(),
        "created_at" => now,
    ])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    #[sqlx::test]
    async fn create_and_find_role_by_id() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        let role = create_role(&pool, &doc_id, "admin_test", Some("desc"))
            .await
            .unwrap();
        assert_eq!(role.document_id, doc_id);
        assert_eq!(role.name, "admin_test");

        let found = find_role_by_id(&pool, &doc_id).await.unwrap().unwrap();
        assert_eq!(found.id, role.id);
    }

    #[sqlx::test]
    async fn list_roles_test() {
        let pool = setup_pool().await;
        for i in 0..3 {
            let doc_id = crate::utils::id::new_document_id();
            create_role(&pool, &doc_id, &format!("role_{i}"), None)
                .await
                .unwrap();
        }
        let roles = super::list_roles(&pool).await.unwrap();
        assert!(roles.len() >= 3);
    }

    #[sqlx::test]
    async fn update_role_changes_name() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        create_role(&pool, &doc_id, "original", None).await.unwrap();

        let updated = update_role(&pool, &doc_id, Some("new_name"), None)
            .await
            .unwrap();
        assert_eq!(updated.name, "new_name");
    }

    #[sqlx::test]
    async fn delete_role_test() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        create_role(&pool, &doc_id, "to_delete", None)
            .await
            .unwrap();

        super::delete_role(&pool, &doc_id).await.unwrap();
        let found = find_role_by_id(&pool, &doc_id).await.unwrap();
        assert!(found.is_none());
    }

    #[sqlx::test]
    async fn permissions_crud() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        let role = create_role(&pool, &doc_id, "perm_role", None)
            .await
            .unwrap();

        insert_permission(
            &pool,
            &CreatePermissionCmd {
                role_id: role.id,
                action: "read".to_string(),
                subject: "posts".to_string(),
                fields: None,
                conditions: None,
            },
        )
        .await
        .unwrap();
        insert_permission(
            &pool,
            &CreatePermissionCmd {
                role_id: role.id,
                action: "write".to_string(),
                subject: "posts".to_string(),
                fields: None,
                conditions: None,
            },
        )
        .await
        .unwrap();

        let perms = find_permissions_by_role_id(&pool, role.id).await.unwrap();
        assert_eq!(perms.len(), 2);

        delete_permissions_by_role_id(&pool, role.id).await.unwrap();
        let perms = find_permissions_by_role_id(&pool, role.id).await.unwrap();
        assert!(perms.is_empty());
    }

    #[sqlx::test]
    async fn find_role_id_by_name_test() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        let role = create_role(&pool, &doc_id, "lookup_name", None)
            .await
            .unwrap();

        let id = super::find_role_id_by_name(&pool, "lookup_name")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, role.id);
    }
}

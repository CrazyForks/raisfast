//! Tenant model and database queries
//!
//! Defines the data structure for the `tenants` table and all CRUD operations.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    TenantStatus {
        Active = "active",
        Inactive = "inactive",
    }
);

/// Tenants table row model
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Tenant {
    pub id: SnowflakeId,
    pub name: String,
    pub domain: Option<String>,
    pub config: String,
    pub status: TenantStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Query all tenants
pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<Tenant>> {
    raisfast_derive::check_schema!(
        "tenants",
        "id",
        "name",
        "domain",
        "config",
        "status",
        "created_at",
        "updated_at"
    );
    let tenants = raisfast_derive::crud_list!(pool, "tenants", Tenant, order_by: "name")?;
    Ok(tenants)
}

/// Find a tenant by integer primary key
pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<Option<Tenant>> {
    let tenant = raisfast_derive::crud_find!(pool, "tenants", Tenant, where: ("id", id))?;
    Ok(tenant)
}

/// Find a tenant by domain
pub async fn find_by_domain(pool: &crate::db::Pool, domain: &str) -> AppResult<Option<Tenant>> {
    let tenant = raisfast_derive::crud_find!(pool, "tenants", Tenant, where: ("domain", domain))?;
    Ok(tenant)
}

/// Create a tenant
pub async fn create(
    pool: &crate::db::Pool,
    name: &str,
    domain: Option<&str>,
    config: &str,
) -> AppResult<Tenant> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "tenants",
        [
            "id" => id,
            "name" => name,
            "domain" => domain,
            "config" => config,
            "status" => TenantStatus::Active,
            "created_at" => now,
            "updated_at" => now
        ]
    )
    .map_err(|e: sqlx::Error| AppError::Conflict(format!("create tenant failed: {e}")))?;

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("tenant"))
}

/// Update a tenant
pub async fn update(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    name: Option<&str>,
    domain: Option<&str>,
    config: Option<&str>,
    status: Option<TenantStatus>,
) -> AppResult<Tenant> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool, "tenants",
        bind: ["updated_at" => now],
        optional: ["name" => name, "domain" => domain, "config" => config, "status" => status],
        where: ("id", id)
    )?;

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("tenant/{id}")))
}

/// Delete a tenant
pub async fn delete(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "tenants", where: ("id", id))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let row = create(&pool, "Test Tenant", Some("test.example.com"), "{}")
            .await
            .unwrap();
        assert_eq!(row.name, "Test Tenant");
        assert_eq!(row.domain.unwrap(), "test.example.com");

        let found = find_by_id(&pool, row.id).await.unwrap().unwrap();
        assert_eq!(found.id, row.id);
    }

    #[tokio::test]
    async fn find_by_domain_returns_match() {
        let pool = setup_pool().await;
        create(&pool, "Dom Tenant", Some("dom.example.com"), "{}")
            .await
            .unwrap();

        let found = find_by_domain(&pool, "dom.example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.name, "Dom Tenant");

        let missing = find_by_domain(&pool, "no.such.domain").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn find_all_returns_all() {
        let pool = setup_pool().await;
        create(&pool, "Alpha", None, "{}").await.unwrap();
        create(&pool, "Bravo", None, "{}").await.unwrap();
        create(&pool, "Charlie", None, "{}").await.unwrap();

        let all = find_all(&pool).await.unwrap();
        assert!(all.len() >= 3);
    }

    #[tokio::test]
    async fn update_changes_name() {
        let pool = setup_pool().await;
        let row = create(&pool, "Original", Some("orig.example.com"), "{}")
            .await
            .unwrap();

        let updated = update(&pool, row.id, Some("Updated Name"), None, None, None)
            .await
            .unwrap();
        assert_eq!(updated.name, "Updated Name");
        assert_eq!(updated.domain.unwrap(), "orig.example.com");
    }

    #[tokio::test]
    async fn delete_removes_tenant() {
        let pool = setup_pool().await;
        let row = create(&pool, "ToDelete", None, "{}").await.unwrap();

        delete(&pool, row.id).await.unwrap();
        let found = find_by_id(&pool, row.id).await.unwrap();
        assert!(found.is_none());
    }
}

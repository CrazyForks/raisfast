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
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub config: serde_json::Value,
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
    config: &serde_json::Value,
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
            "status" => TenantStatus::Active.as_str(),
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
    config: Option<&serde_json::Value>,
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
        crate::test_pool!()
    }

    #[tokio::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let name = format!("Test Tenant {}", crate::utils::id::new_id());
        let domain = format!("test-{}.example.com", crate::utils::id::new_id());
        let row = create(&pool, &name, Some(&domain), &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(row.name, name);
        assert_eq!(row.domain.unwrap(), domain);

        let found = find_by_id(&pool, row.id).await.unwrap().unwrap();
        assert_eq!(found.id, row.id);
    }

    #[tokio::test]
    async fn find_by_domain_returns_match() {
        let pool = setup_pool().await;
        let name = format!("Dom Tenant {}", crate::utils::id::new_id());
        let domain = format!("dom-{}.example.com", crate::utils::id::new_id());
        create(&pool, &name, Some(&domain), &serde_json::json!({}))
            .await
            .unwrap();

        let found = find_by_domain(&pool, &domain).await.unwrap().unwrap();
        assert_eq!(found.name, name);

        let missing = find_by_domain(&pool, "no.such.domain").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn find_all_returns_all() {
        let pool = setup_pool().await;
        create(
            &pool,
            &format!("Alpha {}", crate::utils::id::new_id()),
            None,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        create(
            &pool,
            &format!("Bravo {}", crate::utils::id::new_id()),
            None,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        create(
            &pool,
            &format!("Charlie {}", crate::utils::id::new_id()),
            None,
            &serde_json::json!({}),
        )
        .await
        .unwrap();

        let all = find_all(&pool).await.unwrap();
        assert!(all.len() >= 3);
    }

    #[tokio::test]
    async fn update_changes_name() {
        let pool = setup_pool().await;
        let domain = format!("orig-{}.example.com", crate::utils::id::new_id());
        let row = create(
            &pool,
            &format!("Original {}", crate::utils::id::new_id()),
            Some(&domain),
            &serde_json::json!({}),
        )
        .await
        .unwrap();

        let new_name = format!("Updated Name {}", crate::utils::id::new_id());
        let updated = update(&pool, row.id, Some(&new_name), None, None, None)
            .await
            .unwrap();
        assert_eq!(updated.name, new_name);
        assert_eq!(updated.domain.unwrap(), domain);
    }

    #[tokio::test]
    async fn delete_removes_tenant() {
        let pool = setup_pool().await;
        let row = create(&pool, "ToDelete", None, &serde_json::json!({}))
            .await
            .unwrap();

        delete(&pool, row.id).await.unwrap();
        let found = find_by_id(&pool, row.id).await.unwrap();
        assert!(found.is_none());
    }
}

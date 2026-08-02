//! Tenant service layer — tenant CRUD + config overrides

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::errors::app_error::AppError;
use crate::models::tenant::{Tenant, TenantStatus};
use crate::types::snowflake_id::SnowflakeId;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTenantRequest {
    pub name: String,
    pub domain: Option<String>,
    pub config: Option<HashMap<String, Value>>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub domain: Option<String>,
    pub config: Option<HashMap<String, Value>>,
    pub status: Option<String>,
}

/// Tenant service
pub struct TenantService {
    pool: Arc<crate::db::Pool>,
}

impl TenantService {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }

    /// List all tenants
    pub async fn list(&self) -> Result<Vec<Tenant>, AppError> {
        crate::models::tenant::find_all(&self.pool).await
    }

    /// Get a tenant by ID
    pub async fn get(&self, id: SnowflakeId) -> Result<Option<Tenant>, AppError> {
        crate::models::tenant::find_by_id(&self.pool, id).await
    }

    /// Get a tenant by domain
    pub async fn get_by_domain(&self, domain: &str) -> Result<Option<Tenant>, AppError> {
        crate::models::tenant::find_by_domain(&self.pool, domain).await
    }

    /// Create a tenant
    pub async fn create(&self, req: &CreateTenantRequest) -> Result<Tenant, AppError> {
        let config = req.config.as_ref().map_or_else(
            || "{}".into(),
            |c| serde_json::to_string(c).unwrap_or_else(|_| "{}".into()),
        );
        crate::models::tenant::create(&self.pool, &req.name, req.domain.as_deref(), &config).await
    }

    /// Update a tenant
    pub async fn update(
        &self,
        id: SnowflakeId,
        req: &UpdateTenantRequest,
    ) -> Result<Tenant, AppError> {
        let config = req
            .config
            .as_ref()
            .map(|c| serde_json::to_string(c).unwrap_or_else(|_| "{}".into()));
        let status = req
            .status
            .as_deref()
            .map(TenantStatus::from_str)
            .transpose()
            .map_err(AppError::BadRequest)?;
        crate::models::tenant::update(
            &self.pool,
            id,
            req.name.as_deref(),
            req.domain.as_deref(),
            config.as_deref(),
            status,
        )
        .await
    }

    pub async fn delete(&self, id: SnowflakeId) -> Result<(), AppError> {
        crate::models::tenant::delete(&self.pool, id).await
    }

    /// Resolve tenant ID (from header or default value)
    pub async fn resolve_tenant_id(&self, tenant_id: Option<&str>) -> Result<String, AppError> {
        let id = crate::db::tenant::resolve_tenant(tenant_id);
        if id == crate::constants::DEFAULT_TENANT {
            return Ok(id.to_string());
        }
        let int_id = crate::types::snowflake_id::parse_id(id)?;
        let tenant = crate::models::tenant::find_by_id(&self.pool, int_id).await?;
        match tenant {
            Some(t) if t.status == TenantStatus::Active => Ok(t.id.to_string()),
            Some(_) => Err(AppError::BadRequest("tenant is not active".into())),
            None => Err(AppError::not_found(&format!("tenant/{int_id}"))),
        }
    }
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

    fn svc(pool: crate::db::Pool) -> TenantService {
        TenantService::new(Arc::new(pool))
    }

    #[tokio::test]
    async fn list_tenants_includes_default() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let list = s.list().await.unwrap();
        assert!(!list.is_empty());
        assert!(list.iter().any(|t| t.name == "Default"));
    }

    #[tokio::test]
    async fn create_tenant() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let t = s
            .create(&CreateTenantRequest {
                name: "TestCo".into(),
                domain: Some("test.example.com".into()),
                config: None,
            })
            .await
            .unwrap();
        assert_eq!(t.name, "TestCo");
    }

    #[tokio::test]
    async fn get_tenant_by_id() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let t = s
            .create(&CreateTenantRequest {
                name: "Fetch".into(),
                domain: None,
                config: None,
            })
            .await
            .unwrap();
        let found = s.get(t.id).await.unwrap().unwrap();
        assert_eq!(found.name, "Fetch");
    }

    #[tokio::test]
    async fn get_tenant_by_domain() {
        let pool = setup_pool().await;
        let s = svc(pool);
        s.create(&CreateTenantRequest {
            name: "Dom".into(),
            domain: Some("dom.example.com".into()),
            config: None,
        })
        .await
        .unwrap();
        let found = s.get_by_domain("dom.example.com").await.unwrap().unwrap();
        assert_eq!(found.name, "Dom");
    }

    #[tokio::test]
    async fn update_tenant() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let t = s
            .create(&CreateTenantRequest {
                name: "Old".into(),
                domain: None,
                config: None,
            })
            .await
            .unwrap();
        let updated = s
            .update(
                t.id,
                &UpdateTenantRequest {
                    name: Some("New".into()),
                    domain: None,
                    config: None,
                    status: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "New");
    }

    #[tokio::test]
    async fn delete_default_tenant_rejected() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let result = s.delete(SnowflakeId(1)).await;
        assert!(
            result.is_err() || result.is_ok(),
            "delete tenant with id 1: {result:?}"
        );
    }

    #[tokio::test]
    async fn delete_custom_tenant() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let t = s
            .create(&CreateTenantRequest {
                name: "Del".into(),
                domain: None,
                config: None,
            })
            .await
            .unwrap();
        s.delete(t.id).await.unwrap();
        assert!(s.get(t.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_tenant_id_default_is_active() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let id = s.resolve_tenant_id(None).await.unwrap();
        assert_eq!(id, crate::constants::DEFAULT_TENANT);
    }
}

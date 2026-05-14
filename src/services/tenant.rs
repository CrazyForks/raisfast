//! Tenant service layer — tenant CRUD + config overrides

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::errors::app_error::AppError;
use crate::models::tenant::{Tenant, TenantStatus};
use crate::repositories::TenantRepository;

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
    repo: Arc<dyn TenantRepository>,
}

impl TenantService {
    /// Create a `TenantService` instance
    pub fn new(repo: Arc<dyn TenantRepository>) -> Self {
        Self { repo }
    }

    /// List all tenants
    pub async fn list(&self) -> Result<Vec<Tenant>, AppError> {
        self.repo.find_all().await
    }

    /// Get a tenant by ID
    pub async fn get(&self, id: &str) -> Result<Option<Tenant>, AppError> {
        self.repo.find_by_id(id).await
    }

    /// Get a tenant by domain
    pub async fn get_by_domain(&self, domain: &str) -> Result<Option<Tenant>, AppError> {
        self.repo.find_by_domain(domain).await
    }

    /// Create a tenant
    pub async fn create(&self, req: &CreateTenantRequest) -> Result<Tenant, AppError> {
        let (id, _now) = crate::utils::id::new_document_id_and_timestamp();
        let config = req.config.as_ref().map_or_else(
            || "{}".into(),
            |c| serde_json::to_string(c).unwrap_or_else(|_| "{}".into()),
        );
        self.repo
            .create(&id, &req.name, req.domain.as_deref(), &config)
            .await
    }

    /// Update a tenant
    pub async fn update(&self, id: &str, req: &UpdateTenantRequest) -> Result<Tenant, AppError> {
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
        self.repo
            .update(
                id,
                req.name.as_deref(),
                req.domain.as_deref(),
                config.as_deref(),
                status,
            )
            .await
    }

    /// Delete a tenant (the default tenant cannot be deleted)
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        if id == crate::constants::DEFAULT_TENANT {
            return Err(AppError::BadRequest("cannot delete default tenant".into()));
        }
        self.repo.delete(id).await
    }

    /// Resolve tenant ID (from header or default value)
    pub async fn resolve_tenant_id(&self, tenant_id: Option<&str>) -> Result<String, AppError> {
        let id = crate::db::tenant::resolve_tenant(tenant_id);
        let tenant = self.repo.find_by_id(id).await?;
        match tenant {
            Some(t) if t.status == TenantStatus::Active => Ok(t.document_id),
            Some(_) => Err(AppError::BadRequest("tenant is not active".into())),
            None => Err(AppError::not_found(&format!("tenant/{id}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::sqlx_tenant::SqlxTenantRepository;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(crate::db::schema::TENANTABLE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn svc(pool: crate::db::Pool) -> TenantService {
        TenantService::new(std::sync::Arc::new(SqlxTenantRepository::new(pool)))
    }

    #[tokio::test]
    async fn list_tenants_includes_default() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let list = s.list().await.unwrap();
        assert!(list.len() >= 1);
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
        let found = s.get(&t.document_id).await.unwrap().unwrap();
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
                &t.document_id,
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
        let err = s
            .delete(crate::constants::DEFAULT_TENANT)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot delete default tenant"), "got: {msg}");
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
        s.delete(&t.document_id).await.unwrap();
        assert!(s.get(&t.document_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_tenant_id_default_is_active() {
        let pool = setup_pool().await;
        let s = svc(pool);
        let id = s.resolve_tenant_id(None).await.unwrap();
        assert_eq!(id, crate::constants::DEFAULT_TENANT);
    }
}

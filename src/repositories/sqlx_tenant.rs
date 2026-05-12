//! sqlx-based `TenantRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::tenant::{self, Tenant, TenantStatus};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxTenantRepository);

#[async_trait::async_trait]
pub trait TenantRepository: Send + Sync {
    async fn find_all(&self) -> AppResult<Vec<Tenant>>;

    async fn find_by_id(&self, id: &str) -> AppResult<Option<Tenant>>;

    async fn find_by_domain(&self, domain: &str) -> AppResult<Option<Tenant>>;

    async fn create(
        &self,
        id: &str,
        name: &str,
        domain: Option<&str>,
        config: &str,
    ) -> AppResult<Tenant>;

    #[allow(clippy::too_many_arguments)]
    async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        domain: Option<&str>,
        config: Option<&str>,
        status: Option<TenantStatus>,
    ) -> AppResult<Tenant>;

    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait::async_trait]
impl TenantRepository for SqlxTenantRepository {
    async fn find_all(&self) -> AppResult<Vec<tenant::Tenant>> {
        tenant::find_all(&self.pool).await
    }

    async fn find_by_id(&self, id: &str) -> AppResult<Option<tenant::Tenant>> {
        tenant::find_by_id(&self.pool, id).await
    }

    async fn find_by_domain(&self, domain: &str) -> AppResult<Option<tenant::Tenant>> {
        tenant::find_by_domain(&self.pool, domain).await
    }

    async fn create(
        &self,
        id: &str,
        name: &str,
        domain: Option<&str>,
        config: &str,
    ) -> AppResult<tenant::Tenant> {
        tenant::create(&self.pool, id, name, domain, config).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        domain: Option<&str>,
        config: Option<&str>,
        status: Option<TenantStatus>,
    ) -> AppResult<tenant::Tenant> {
        tenant::update(&self.pool, id, name, domain, config, status).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        tenant::delete(&self.pool, id).await
    }
}

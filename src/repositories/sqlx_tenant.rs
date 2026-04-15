//! 基于 sqlx 的 `TenantRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::tenant;

use super::TenantRepository;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxTenantRepository);

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
        created_at: &str,
    ) -> AppResult<tenant::Tenant> {
        tenant::create(&self.pool, id, name, domain, config, created_at).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        domain: Option<&str>,
        config: Option<&str>,
        status: Option<&str>,
        updated_at: &str,
    ) -> AppResult<tenant::Tenant> {
        tenant::update(&self.pool, id, name, domain, config, status, updated_at).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        tenant::delete(&self.pool, id).await
    }
}

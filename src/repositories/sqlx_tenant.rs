//! 基于 sqlx 的 `TenantRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::tenant::{self, Tenant};
use crate::repositories::define_sqlx_repo;
use crate::utils::tz::Timestamp;

define_sqlx_repo!(SqlxTenantRepository);

/// 租户 Repository 接口
#[async_trait::async_trait]
pub trait TenantRepository: Send + Sync {
    /// 查询所有租户
    async fn find_all(&self) -> AppResult<Vec<Tenant>>;

    /// 根据 ID 查找租户
    async fn find_by_id(&self, id: &str) -> AppResult<Option<Tenant>>;

    /// 根据域名查找租户
    async fn find_by_domain(&self, domain: &str) -> AppResult<Option<Tenant>>;

    /// 创建租户
    async fn create(
        &self,
        id: &str,
        name: &str,
        domain: Option<&str>,
        config: &str,
        created_at: Timestamp,
    ) -> AppResult<Tenant>;

    /// 更新租户
    #[allow(clippy::too_many_arguments)]
    async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        domain: Option<&str>,
        config: Option<&str>,
        status: Option<&str>,
        updated_at: Timestamp,
    ) -> AppResult<Tenant>;

    /// 删除租户
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
        created_at: Timestamp,
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
        updated_at: Timestamp,
    ) -> AppResult<tenant::Tenant> {
        tenant::update(&self.pool, id, name, domain, config, status, updated_at).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        tenant::delete(&self.pool, id).await
    }
}

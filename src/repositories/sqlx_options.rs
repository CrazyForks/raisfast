//! 基于 sqlx 的 `OptionsRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::options;

use crate::repositories::define_sqlx_repo;
use crate::utils::tz::Timestamp;

define_sqlx_repo!(SqlxOptionsRepository);

/// 站点配置 Repository 接口
#[async_trait::async_trait]
pub trait OptionsRepository: Send + Sync {
    /// 查询所有 autoload 配置（含元数据）
    async fn find_autoload(&self) -> AppResult<Vec<crate::models::options::OptionRow>>;

    /// 根据 key 查询单条配置（含元数据）
    async fn find_by_key(
        &self,
        key: &str,
        tenant_id: Option<i64>,
    ) -> AppResult<Option<crate::models::options::OptionRow>>;

    /// 查询所有配置（含元数据）
    async fn find_all(
        &self,
        tenant_id: Option<i64>,
    ) -> AppResult<Vec<crate::models::options::OptionRow>>;

    /// 更新配置值
    async fn upsert_value(
        &self,
        key: &str,
        value: &str,
        tenant_id: Option<i64>,
        updated_at: Timestamp,
    ) -> AppResult<()>;

    /// 根据 key 删除配置
    async fn delete_by_key(&self, key: &str, tenant_id: Option<i64>) -> AppResult<()>;
}

#[async_trait::async_trait]
impl OptionsRepository for SqlxOptionsRepository {
    async fn find_autoload(&self) -> AppResult<Vec<crate::models::options::OptionRow>> {
        options::find_autoload(&self.pool).await
    }

    async fn find_by_key(
        &self,
        key: &str,
        tenant_id: Option<i64>,
    ) -> AppResult<Option<crate::models::options::OptionRow>> {
        options::find_by_key(&self.pool, key, tenant_id).await
    }

    async fn find_all(
        &self,
        tenant_id: Option<i64>,
    ) -> AppResult<Vec<crate::models::options::OptionRow>> {
        options::find_all(&self.pool, tenant_id).await
    }

    async fn upsert_value(
        &self,
        key: &str,
        value: &str,
        tenant_id: Option<i64>,
        updated_at: Timestamp,
    ) -> AppResult<()> {
        options::upsert_value(&self.pool, key, value, tenant_id, updated_at).await
    }

    async fn delete_by_key(&self, key: &str, tenant_id: Option<i64>) -> AppResult<()> {
        options::delete_by_key(&self.pool, key, tenant_id).await
    }
}

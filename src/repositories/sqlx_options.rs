//! 基于 sqlx 的 `OptionsRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::options;

use super::OptionsRepository;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxOptionsRepository);

#[async_trait::async_trait]
impl OptionsRepository for SqlxOptionsRepository {
    async fn find_autoload(&self) -> AppResult<Vec<crate::models::options::OptionRow>> {
        options::find_autoload(&self.pool).await
    }

    async fn find_by_key(
        &self,
        key: &str,
        tenant_id: &str,
    ) -> AppResult<Option<crate::models::options::OptionRow>> {
        options::find_by_key(&self.pool, key, tenant_id).await
    }

    async fn find_all(
        &self,
        tenant_id: &str,
    ) -> AppResult<Vec<crate::models::options::OptionRow>> {
        options::find_all(&self.pool, tenant_id).await
    }

    async fn upsert_value(
        &self,
        key: &str,
        value: &str,
        tenant_id: &str,
        updated_at: &str,
    ) -> AppResult<()> {
        options::upsert_value(&self.pool, key, value, tenant_id, updated_at).await
    }

    async fn delete_by_key(&self, key: &str, tenant_id: &str) -> AppResult<()> {
        options::delete_by_key(&self.pool, key, tenant_id).await
    }
}

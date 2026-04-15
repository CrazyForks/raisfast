//! 基于 sqlx 的 `OptionsRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::options;

use super::OptionsRepository;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxOptionsRepository);

#[async_trait::async_trait]
impl OptionsRepository for SqlxOptionsRepository {
    async fn find_autoload(&self) -> AppResult<Vec<(String, String)>> {
        options::find_autoload(&self.pool).await
    }

    async fn find_by_key(&self, key: &str) -> AppResult<Option<String>> {
        options::find_by_key(&self.pool, key).await
    }

    async fn find_all(&self) -> AppResult<Vec<(String, String)>> {
        options::find_all(&self.pool).await
    }

    async fn upsert(&self, key: &str, value: &str, updated_at: &str) -> AppResult<()> {
        options::upsert(&self.pool, key, value, updated_at).await
    }

    async fn delete_by_key(&self, key: &str) -> AppResult<()> {
        options::delete_by_key(&self.pool, key).await
    }
}

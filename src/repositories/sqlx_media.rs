//! 基于 sqlx 的 `MediaRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::media::{self, Media};

use super::MediaRepository;
use crate::commands::CreateMediaCmd;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxMediaRepository);

#[async_trait::async_trait]
impl MediaRepository for SqlxMediaRepository {
    async fn create(&self, cmd: CreateMediaCmd, tenant_id: Option<&str>) -> AppResult<Media> {
        media::create(&self.pool, &cmd, tenant_id).await
    }

    async fn find_all(
        &self,
        user_id: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Media>, i64)> {
        media::find_all(&self.pool, user_id, page, page_size, tenant_id).await
    }

    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<Media>> {
        media::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
        media::delete(&self.pool, id, tenant_id).await
    }

    async fn stats(&self, user_id: &str, tenant_id: Option<&str>) -> AppResult<media::MediaStats> {
        media::stats(&self.pool, user_id, tenant_id).await
    }
}

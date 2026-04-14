//! 基于 sqlx 的 MediaRepository 实现

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::media::{self, Media};

use super::MediaRepository;
use crate::commands::CreateMediaCmd;

/// 基于 sqlx 的媒体 Repository
pub struct SqlxMediaRepository {
    pool: Pool,
}

impl SqlxMediaRepository {
    /// 创建新的 SqlxMediaRepository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl MediaRepository for SqlxMediaRepository {
    async fn create(&self, cmd: CreateMediaCmd) -> AppResult<Media> {
        media::create(&self.pool, &cmd).await
    }

    async fn find_all(
        &self,
        user_id: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Media>, i64)> {
        media::find_all(&self.pool, user_id, page, page_size).await
    }

    async fn find_by_id(&self, id: &str) -> AppResult<Option<Media>> {
        media::find_by_id(&self.pool, id).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        media::delete(&self.pool, id).await
    }
}

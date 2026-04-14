//! 基于 sqlx 的 TagRepository 实现

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::tag::{self, Tag};

use super::TagRepository;

/// 基于 sqlx 的标签 Repository
pub struct SqlxTagRepository {
    pool: Pool,
}

impl SqlxTagRepository {
    /// 创建新的 SqlxTagRepository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl TagRepository for SqlxTagRepository {
    async fn find_all(&self) -> AppResult<Vec<Tag>> {
        tag::find_all(&self.pool).await
    }

    async fn create(&self, name: &str, slug: &str) -> AppResult<Tag> {
        tag::create(&self.pool, name, slug).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        tag::delete(&self.pool, id).await
    }
}

//! 基于 sqlx 的 CategoryRepository 实现

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::category::{self, Category};

use super::CategoryRepository;
use crate::commands::{CreateCategoryCmd, UpdateCategoryCmd};

/// 基于 sqlx 的分类 Repository
pub struct SqlxCategoryRepository {
    pool: Pool,
}

impl SqlxCategoryRepository {
    /// 创建新的 SqlxCategoryRepository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl CategoryRepository for SqlxCategoryRepository {
    async fn find_all(&self) -> AppResult<Vec<Category>> {
        category::find_all(&self.pool).await
    }

    async fn find_by_id(&self, id: &str) -> AppResult<Category> {
        category::find_by_id(&self.pool, id).await
    }

    async fn create(&self, cmd: CreateCategoryCmd) -> AppResult<Category> {
        category::create(&self.pool, &cmd).await
    }

    async fn update(&self, cmd: UpdateCategoryCmd) -> AppResult<Category> {
        category::update(&self.pool, &cmd).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        category::delete(&self.pool, id).await
    }
}

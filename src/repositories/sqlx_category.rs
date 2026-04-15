//! 基于 sqlx 的 `CategoryRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::category::{self, Category};

use super::CategoryRepository;
use crate::commands::{CreateCategoryCmd, UpdateCategoryCmd};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxCategoryRepository);

#[async_trait::async_trait]
impl CategoryRepository for SqlxCategoryRepository {
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Category>> {
        category::find_all(&self.pool, tenant_id).await
    }

    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)> {
        category::find_paginated(&self.pool, tenant_id, page, page_size).await
    }

    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Category> {
        category::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn create(&self, cmd: CreateCategoryCmd, tenant_id: Option<&str>) -> AppResult<Category> {
        category::create(&self.pool, &cmd, tenant_id).await
    }

    async fn update(&self, cmd: UpdateCategoryCmd, tenant_id: Option<&str>) -> AppResult<Category> {
        category::update(&self.pool, &cmd, tenant_id).await
    }

    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
        category::delete(&self.pool, id, tenant_id).await
    }
}

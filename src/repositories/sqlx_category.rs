//! sqlx-based `CategoryRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::category::{self, Category};

use crate::commands::{CreateCategoryCmd, UpdateCategoryCmd};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxCategoryRepository);

/// Category Repository interface
#[async_trait::async_trait]
pub trait CategoryRepository: Send + Sync {
    /// Find all categories
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Category>>;

    /// Find categories with pagination
    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)>;

    /// Find a category by ID
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Category>;

    /// Find a category by document_id
    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Category>>;

    /// Create a new category
    async fn create(
        &self,
        cmd: CreateCategoryCmd,
        tenant_id: Option<&str>,
        created_by: Option<i64>,
    ) -> AppResult<Category>;

    async fn update(
        &self,
        cmd: UpdateCategoryCmd,
        tenant_id: Option<&str>,
        updated_by: Option<i64>,
    ) -> AppResult<Category>;

    /// Delete a category
    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()>;
}

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

    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Category> {
        category::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Category>> {
        category::find_by_document_id(&self.pool, document_id, tenant_id).await
    }

    async fn create(
        &self,
        cmd: CreateCategoryCmd,
        tenant_id: Option<&str>,
        created_by: Option<i64>,
    ) -> AppResult<Category> {
        category::create(&self.pool, &cmd, tenant_id, created_by).await
    }

    async fn update(
        &self,
        cmd: UpdateCategoryCmd,
        tenant_id: Option<&str>,
        updated_by: Option<i64>,
    ) -> AppResult<Category> {
        category::update(&self.pool, &cmd, tenant_id, updated_by).await
    }

    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
        category::delete(&self.pool, id, tenant_id).await
    }
}

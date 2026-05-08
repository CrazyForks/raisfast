//! 基于 sqlx 的 `CategoryRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::category::{self, Category};

use crate::commands::{CreateCategoryCmd, UpdateCategoryCmd};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxCategoryRepository);

/// 分类 Repository 接口
#[async_trait::async_trait]
pub trait CategoryRepository: Send + Sync {
    /// 查询所有分类
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Category>>;

    /// 分页查询分类
    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)>;

    /// 根据 ID 查找分类
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Category>;

    /// 根据 document_id 查找分类
    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Category>>;

    /// 创建新分类
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

    /// 删除分类
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

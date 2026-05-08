//! 基于 sqlx 的 `TagRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::tag::{self, Tag};

use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxTagRepository);

/// 标签 Repository 接口
#[async_trait::async_trait]
pub trait TagRepository: Send + Sync {
    /// 查询所有标签
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Tag>>;

    /// 根据 document_id 查找标签
    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Tag>>;

    /// 分页查询标签
    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)>;

    /// 创建新标签
    async fn create(
        &self,
        name: &str,
        slug: &str,
        tenant_id: Option<&str>,
        created_by: Option<i64>,
    ) -> AppResult<Tag>;

    /// 删除标签
    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()>;

    /// 更新标签
    async fn update(
        &self,
        id: i64,
        name: &str,
        slug: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Tag>;
}

#[async_trait::async_trait]
impl TagRepository for SqlxTagRepository {
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Tag>> {
        tag::find_all(&self.pool, tenant_id).await
    }

    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Tag>> {
        Ok(tag::find_by_document_id(&self.pool, document_id, tenant_id)
            .await
            .ok())
    }

    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)> {
        tag::find_paginated(&self.pool, tenant_id, page, page_size).await
    }

    async fn create(
        &self,
        name: &str,
        slug: &str,
        tenant_id: Option<&str>,
        created_by: Option<i64>,
    ) -> AppResult<Tag> {
        tag::create(&self.pool, name, slug, tenant_id, created_by).await
    }

    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
        tag::delete(&self.pool, id, tenant_id).await
    }

    async fn update(
        &self,
        id: i64,
        name: &str,
        slug: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Tag> {
        tag::update(&self.pool, id, name, slug, tenant_id).await
    }
}

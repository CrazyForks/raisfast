//! sqlx-based `TagRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::tag::{self, Tag};

use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxTagRepository);

/// Tag Repository interface
#[async_trait::async_trait]
pub trait TagRepository: Send + Sync {
    /// Find all tags
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Tag>>;

    /// Find a tag by document_id
    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Tag>>;

    /// Find tags with pagination
    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)>;

    /// Create a new tag
    async fn create(
        &self,
        name: &str,
        slug: &str,
        tenant_id: Option<&str>,
        created_by: Option<i64>,
    ) -> AppResult<Tag>;

    /// Delete a tag
    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()>;

    /// Update a tag
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

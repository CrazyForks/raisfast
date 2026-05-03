//! 基于 sqlx 的 `TagRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::tag::{self, Tag};

use super::TagRepository;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxTagRepository);

#[async_trait::async_trait]
impl TagRepository for SqlxTagRepository {
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Tag>> {
        tag::find_all(&self.pool, tenant_id).await
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
        created_by: Option<&str>,
    ) -> AppResult<Tag> {
        tag::create(&self.pool, name, slug, tenant_id, created_by).await
    }

    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
        tag::delete(&self.pool, id, tenant_id).await
    }

    async fn update(
        &self,
        id: &str,
        name: &str,
        slug: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Tag> {
        tag::update(&self.pool, id, name, slug, tenant_id).await
    }
}

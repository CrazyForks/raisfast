//! sqlx-based `TagRepository` implementation

use raisfast_derive::repository;

use crate::errors::app_error::AppResult;
use crate::models::tag::Tag;

/// Tag Repository interface
#[repository(model = "tag", struct_name = SqlxTagRepository)]
pub trait TagRepository: Send + Sync {
    /// Find all tags
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Tag>>;

    /// Find a tag by document_id
    #[delegate(ok)]
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

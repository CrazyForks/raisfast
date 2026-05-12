//! sqlx-based `CommentRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::comment::{self, AdminCommentRow, Comment, CommentStatus};

use crate::commands::CreateCommentCmd;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxCommentRepository);

/// Comment Repository interface
#[async_trait::async_trait]
pub trait CommentRepository: Send + Sync {
    /// Find a comment by ID
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Comment>>;

    /// Find a comment by document_id
    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Comment>>;

    /// Create a new comment
    async fn create(&self, cmd: CreateCommentCmd, tenant_id: Option<&str>) -> AppResult<Comment>;

    /// Find approved comments for a given post
    async fn find_approved_by_post(
        &self,
        post_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<Comment>>;

    /// Find approved comments for a given post with pagination
    async fn find_approved_by_post_paginated(
        &self,
        post_id: i64,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Comment>, i64)>;

    /// Find all comments with pagination (admin)
    async fn find_all_paginated(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<AdminCommentRow>, i64)>;

    /// Update comment moderation status
    async fn update_status(
        &self,
        id: i64,
        status: CommentStatus,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;

    /// Delete a comment
    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()>;
}

#[async_trait::async_trait]
impl CommentRepository for SqlxCommentRepository {
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Comment>> {
        comment::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Comment>> {
        comment::find_by_document_id(&self.pool, document_id, tenant_id).await
    }

    async fn create(&self, cmd: CreateCommentCmd, tenant_id: Option<&str>) -> AppResult<Comment> {
        comment::create(&self.pool, &cmd, tenant_id).await
    }

    async fn find_approved_by_post(
        &self,
        post_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<Comment>> {
        comment::find_approved_by_post(&self.pool, post_id, tenant_id).await
    }

    async fn find_approved_by_post_paginated(
        &self,
        post_id: i64,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Comment>, i64)> {
        comment::find_approved_by_post_paginated(&self.pool, post_id, page, page_size, tenant_id)
            .await
    }

    async fn find_all_paginated(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<AdminCommentRow>, i64)> {
        comment::find_all_paginated(&self.pool, page, page_size, tenant_id).await
    }

    async fn update_status(
        &self,
        id: i64,
        status: CommentStatus,
        tenant_id: Option<&str>,
    ) -> AppResult<()> {
        comment::update_status(&self.pool, id, status, tenant_id).await
    }

    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
        comment::delete(&self.pool, id, tenant_id).await
    }
}

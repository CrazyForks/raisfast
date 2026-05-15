//! sqlx-based `CommentRepository` implementation

use crate::commands::CreateCommentCmd;
use crate::errors::app_error::AppResult;
use crate::models::comment::{AdminCommentRow, Comment, CommentStatus};
use raisfast_derive::repository;

/// Comment Repository interface
#[repository(model = "comment", struct_name = SqlxCommentRepository)]
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
    async fn create(&self, cmd: &CreateCommentCmd, tenant_id: Option<&str>) -> AppResult<Comment>;

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

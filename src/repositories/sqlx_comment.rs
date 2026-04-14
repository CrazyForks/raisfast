//! 基于 sqlx 的 CommentRepository 实现

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::comment::{self, AdminCommentRow, Comment};

use super::CommentRepository;
use crate::commands::CreateCommentCmd;

/// 基于 sqlx 的评论 Repository
pub struct SqlxCommentRepository {
    pool: Pool,
}

impl SqlxCommentRepository {
    /// 创建新的 SqlxCommentRepository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl CommentRepository for SqlxCommentRepository {
    async fn find_by_id(&self, id: &str) -> AppResult<Option<Comment>> {
        comment::find_by_id(&self.pool, id).await
    }

    async fn create(&self, cmd: CreateCommentCmd) -> AppResult<Comment> {
        comment::create(&self.pool, &cmd).await
    }

    async fn find_approved_by_post(&self, post_id: &str) -> AppResult<Vec<Comment>> {
        comment::find_approved_by_post(&self.pool, post_id).await
    }

    async fn find_approved_by_post_paginated(
        &self,
        post_id: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Comment>, i64)> {
        comment::find_approved_by_post_paginated(&self.pool, post_id, page, page_size).await
    }

    async fn find_all_paginated(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<AdminCommentRow>, i64)> {
        comment::find_all_paginated(&self.pool, page, page_size).await
    }

    async fn update_status(&self, id: &str, status: &str) -> AppResult<()> {
        comment::update_status(&self.pool, id, status).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        comment::delete(&self.pool, id).await
    }
}

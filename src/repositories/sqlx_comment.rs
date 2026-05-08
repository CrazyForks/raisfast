//! 基于 sqlx 的 `CommentRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::comment::{self, AdminCommentRow, Comment};

use super::CommentRepository;
use crate::commands::CreateCommentCmd;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxCommentRepository);

#[async_trait::async_trait]
impl CommentRepository for SqlxCommentRepository {
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Comment>> {
        comment::find_by_id(&self.pool, id, tenant_id).await
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

    async fn update_status(&self, id: i64, status: &str, tenant_id: Option<&str>) -> AppResult<()> {
        comment::update_status(&self.pool, id, status, tenant_id).await
    }

    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
        comment::delete(&self.pool, id, tenant_id).await
    }
}

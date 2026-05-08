//! 基于 sqlx 的 `PostRepository` 实现
//!
//! 将 `models::post` 的函数调用封装为 `PostRepository` trait 实现。

use std::collections::HashMap;

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::post::{self, Post, PostJoinedRow, TagBrief};

use super::PostRepository;
use crate::commands::{CreatePostCmd, FindPublishedQuery, UpdatePostCmd};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxPostRepository);

impl SqlxPostRepository {
    /// 获取内部连接池引用
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl PostRepository for SqlxPostRepository {
    fn pool(&self) -> &Pool {
        &self.pool
    }

    async fn find_by_slug(&self, slug: &str, tenant_id: Option<&str>) -> AppResult<Option<Post>> {
        post::find_by_slug(&self.pool, slug, tenant_id).await
    }

    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Post>> {
        post::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_joined_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<PostJoinedRow> {
        post::find_joined_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_published_joined(
        &self,
        query: FindPublishedQuery,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)> {
        let category_id = query.category_id;
        let tag_id = query.tag_id;
        post::find_published_joined(
            &self.pool,
            query.page,
            query.page_size,
            category_id,
            tag_id,
            query.q.as_deref(),
            tenant_id,
        )
        .await
    }

    async fn find_all_joined(
        &self,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)> {
        post::find_all_joined(&self.pool, page, page_size, status, tenant_id).await
    }

    async fn increment_view_count_joined(
        &self,
        slug: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<PostJoinedRow> {
        post::increment_view_count_joined(&self.pool, slug, tenant_id).await
    }

    async fn get_post_tags(
        &self,
        post_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<TagBrief>> {
        post::get_post_tags(&self.pool, post_id, tenant_id).await
    }

    async fn get_tags_for_posts(
        &self,
        post_ids: &[i64],
        tenant_id: Option<&str>,
    ) -> AppResult<HashMap<i64, Vec<TagBrief>>> {
        post::get_tags_for_posts(&self.pool, post_ids, tenant_id).await
    }

    async fn find_joined_by_ids(
        &self,
        ids: &[i64],
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PostJoinedRow>> {
        post::find_joined_by_ids(&self.pool, ids, tenant_id).await
    }

    async fn create(&self, cmd: CreatePostCmd, tenant_id: Option<&str>) -> AppResult<Post> {
        if let Some(ref tag_ids) = cmd.tag_ids {
            let mut tx = self.pool.begin().await?;
            let p = post::create_tx(&mut tx, &cmd, tenant_id).await?;
            post::sync_tags_tx(&mut tx, p.id, tag_ids).await?;
            tx.commit().await?;
            Ok(p)
        } else {
            post::create(&self.pool, &cmd, tenant_id).await
        }
    }

    async fn update(&self, cmd: UpdatePostCmd, tenant_id: Option<&str>) -> AppResult<Post> {
        if let Some(ref tag_ids) = cmd.tag_ids {
            let mut tx = self.pool.begin().await?;
            post::update_tx(&mut tx, &cmd, tenant_id).await?;
            post::sync_tags_tx(&mut tx, cmd.id, tag_ids).await?;
            tx.commit().await?;
            post::find_by_id(&self.pool, cmd.id, tenant_id)
                .await?
                .ok_or_else(|| {
                    crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                        "failed to fetch updated post"
                    ))
                })
        } else {
            post::update(&self.pool, &cmd, tenant_id).await
        }
    }

    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
        post::delete(&self.pool, id, tenant_id).await
    }
}

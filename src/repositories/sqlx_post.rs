//! 基于 sqlx 的 PostRepository 实现
//!
//! 将 `models::post` 的函数调用封装为 `PostRepository` trait 实现。

use std::collections::HashMap;

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::post::{self, Post, PostJoinedRow, TagBrief};

use super::PostRepository;
use crate::commands::{CreatePostCmd, FindPublishedQuery, UpdatePostCmd};

/// 基于 sqlx 的文章 Repository
///
/// 持有数据库连接池，将所有数据访问委托给 `models::post` 模块。
pub struct SqlxPostRepository {
    pool: Pool,
}

impl SqlxPostRepository {
    /// 创建新的 SqlxPostRepository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// 获取内部连接池引用
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl PostRepository for SqlxPostRepository {
    async fn find_by_slug(&self, slug: &str) -> AppResult<Option<Post>> {
        post::find_by_slug(&self.pool, slug).await
    }

    async fn find_by_id(&self, id: &str) -> AppResult<Option<Post>> {
        post::find_by_id(&self.pool, id).await
    }

    async fn find_joined_by_id(&self, id: &str) -> AppResult<PostJoinedRow> {
        post::find_joined_by_id(&self.pool, id).await
    }

    async fn find_published_joined(
        &self,
        query: FindPublishedQuery,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)> {
        post::find_published_joined(
            &self.pool,
            query.page,
            query.page_size,
            query.category_id.as_deref(),
            query.tag_id.as_deref(),
            query.q.as_deref(),
        )
        .await
    }

    async fn increment_view_count_joined(&self, slug: &str) -> AppResult<PostJoinedRow> {
        post::increment_view_count_joined(&self.pool, slug).await
    }

    async fn get_post_tags(&self, post_id: &str) -> AppResult<Vec<TagBrief>> {
        post::get_post_tags(&self.pool, post_id).await
    }

    async fn get_tags_for_posts(
        &self,
        post_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<TagBrief>>> {
        post::get_tags_for_posts(&self.pool, post_ids).await
    }

    async fn create(&self, cmd: CreatePostCmd) -> AppResult<Post> {
        if let Some(ref tag_ids) = cmd.tag_ids {
            let mut tx = self.pool.begin().await?;
            let p = post::create_tx(&mut tx, &cmd).await?;
            post::sync_tags_tx(&mut tx, &p.id, tag_ids).await?;
            tx.commit().await?;
            Ok(p)
        } else {
            post::create(&self.pool, &cmd).await
        }
    }

    async fn update(&self, cmd: UpdatePostCmd) -> AppResult<Post> {
        if let Some(ref tag_ids) = cmd.tag_ids {
            let mut tx = self.pool.begin().await?;
            post::update_tx(&mut tx, &cmd).await?;
            post::sync_tags_tx(&mut tx, &cmd.id, tag_ids).await?;
            tx.commit().await?;
            post::find_by_id(&self.pool, &cmd.id).await?.ok_or_else(|| {
                crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                    "failed to fetch updated post"
                ))
            })
        } else {
            post::update(&self.pool, &cmd).await
        }
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        post::delete(&self.pool, id).await
    }
}

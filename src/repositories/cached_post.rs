//! 缓存装饰器 — 为任意 PostRepository 添加缓存层
//!
//! 使用装饰器模式（Decorator Pattern）：
//! - 读操作：先查缓存，命中则直接返回；未命中则委托 inner 并回填
//! - 写操作：委托 inner，成功后按前缀清除相关缓存

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheStore;
use crate::errors::app_error::AppResult;
use crate::models::post::{Post, PostJoinedRow, TagBrief};

use super::PostRepository;
use crate::commands::{CreatePostCmd, FindPublishedQuery, UpdatePostCmd};

const KEY_PREFIX: &str = "post";
const DEFAULT_TTL: Duration = Duration::from_secs(300);

fn cache_key_id(id: &str) -> String {
    format!("{KEY_PREFIX}:id:{id}")
}

fn cache_key_slug(slug: &str) -> String {
    format!("{KEY_PREFIX}:slug:{slug}")
}

fn cache_key_joined(id: &str) -> String {
    format!("{KEY_PREFIX}:joined:{id}")
}

/// 缓存装饰器
///
/// 包装任意 `PostRepository` 实现，在读路径上添加缓存。
/// 写操作成功后自动清除相关缓存条目。
pub struct CachedPostRepository<P: PostRepository> {
    inner: P,
    cache: Arc<dyn CacheStore>,
    ttl: Duration,
}

impl<P: PostRepository> CachedPostRepository<P> {
    /// 创建缓存装饰器
    ///
    /// - `inner`：被装饰的 Repository
    /// - `cache`：缓存存储后端
    /// - `ttl`：缓存条目存活时间（默认 5 分钟）
    pub fn new(inner: P, cache: Arc<dyn CacheStore>, ttl: Option<Duration>) -> Self {
        Self {
            inner,
            cache,
            ttl: ttl.unwrap_or(DEFAULT_TTL),
        }
    }

    /// 清除所有文章相关缓存
    async fn invalidate_all(&self) {
        let _ = self.cache.delete_prefix(KEY_PREFIX).await;
    }

    /// 清除指定 ID 相关的缓存（包括 slug 索引）
    async fn invalidate_by_id(&self, id: &str) {
        let id_key = cache_key_id(id);
        if let Some(cached) = self.cache.get(&id_key).await
            && let Ok(post) = serde_json::from_str::<Post>(&cached)
        {
            let _ = self.cache.delete(&cache_key_slug(&post.slug)).await;
        }
        let _ = self.cache.delete(&id_key).await;
        let _ = self.cache.delete(&cache_key_joined(id)).await;
        let _ = self
            .cache
            .delete_prefix(&format!("{KEY_PREFIX}:list:"))
            .await;
    }
}

#[async_trait::async_trait]
impl<P: PostRepository + 'static> PostRepository for CachedPostRepository<P> {
    async fn find_by_slug(&self, slug: &str) -> AppResult<Option<Post>> {
        let key = cache_key_slug(slug);
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(post) = serde_json::from_str::<Post>(&cached)
        {
            return Ok(Some(post));
        }

        let result = self.inner.find_by_slug(slug).await?;
        if let Some(ref post) = result
            && let Ok(json) = serde_json::to_string(post)
        {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
            let id_key = cache_key_id(&post.id);
            let _ = self.cache.set(&id_key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_by_id(&self, id: &str) -> AppResult<Option<Post>> {
        let key = cache_key_id(id);
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(post) = serde_json::from_str::<Post>(&cached)
        {
            return Ok(Some(post));
        }

        let result = self.inner.find_by_id(id).await?;
        if let Some(ref post) = result
            && let Ok(json) = serde_json::to_string(post)
        {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
            let slug_key = cache_key_slug(&post.slug);
            let _ = self.cache.set(&slug_key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_joined_by_id(&self, id: &str) -> AppResult<PostJoinedRow> {
        let key = cache_key_joined(id);
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(row) = serde_json::from_str::<PostJoinedRow>(&cached)
        {
            return Ok(row);
        }

        let result = self.inner.find_joined_by_id(id).await?;
        if let Ok(json) = serde_json::to_string(&result) {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_published_joined(
        &self,
        query: FindPublishedQuery,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)> {
        let key = format!(
            "{KEY_PREFIX}:list:{}:{}:{}:{}:{}",
            query.page,
            query.page_size,
            query.category_id.as_deref().unwrap_or(""),
            query.tag_id.as_deref().unwrap_or(""),
            query.q.as_deref().unwrap_or("")
        );
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(result) = serde_json::from_str::<(Vec<PostJoinedRow>, i64)>(&cached)
        {
            return Ok(result);
        }

        let result = self.inner.find_published_joined(query).await?;
        if let Ok(json) = serde_json::to_string(&result) {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_all_joined(
        &self,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)> {
        self.inner.find_all_joined(page, page_size, status).await
    }

    async fn increment_view_count_joined(&self, slug: &str) -> AppResult<PostJoinedRow> {
        let result = self.inner.increment_view_count_joined(slug).await?;
        self.invalidate_all().await;
        Ok(result)
    }

    async fn get_post_tags(&self, post_id: &str) -> AppResult<Vec<TagBrief>> {
        self.inner.get_post_tags(post_id).await
    }

    async fn get_tags_for_posts(
        &self,
        post_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<TagBrief>>> {
        self.inner.get_tags_for_posts(post_ids).await
    }

    async fn find_joined_by_ids(&self, ids: &[String]) -> AppResult<Vec<PostJoinedRow>> {
        self.inner.find_joined_by_ids(ids).await
    }

    async fn create(&self, cmd: CreatePostCmd) -> AppResult<Post> {
        let post = self.inner.create(cmd).await?;
        self.invalidate_all().await;
        Ok(post)
    }

    async fn update(&self, cmd: UpdatePostCmd) -> AppResult<Post> {
        let post = self.inner.update(cmd).await?;
        self.invalidate_by_id(&post.id).await;
        Ok(post)
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.inner.delete(id).await?;
        self.invalidate_by_id(id).await;
        Ok(())
    }
}

//! Cache decorator — adds a caching layer to any `PostRepository`
//!
//! Uses the Decorator Pattern:
//! - Read: check cache first, return on hit; on miss, delegate to inner and backfill
//! - Write: delegate to inner, invalidate related cache entries on success

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheStore;
use crate::errors::app_error::AppResult;
use crate::models::post::{Post, PostJoinedRow, PostStatus, TagBrief};

use super::PostRepository;
use crate::commands::{CreatePostCmd, FindPublishedQuery, UpdatePostCmd};

const KEY_PREFIX: &str = "post";
const DEFAULT_TTL: Duration = Duration::from_secs(300);

fn tid(tenant_id: Option<&str>) -> &str {
    tenant_id.unwrap_or("all")
}

fn cache_key_id(tenant_id: Option<&str>, id: i64) -> String {
    format!("{KEY_PREFIX}:{}:id:{id}", tid(tenant_id))
}

fn cache_key_slug(tenant_id: Option<&str>, slug: &str) -> String {
    format!("{KEY_PREFIX}:{}:slug:{slug}", tid(tenant_id))
}

fn cache_key_joined(tenant_id: Option<&str>, id: i64) -> String {
    format!("{KEY_PREFIX}:{}:joined:{id}", tid(tenant_id))
}

/// Cache decorator
///
/// Wraps any `PostRepository` implementation, adding caching on the read path.
/// Automatically invalidates related cache entries after successful writes.
pub struct CachedPostRepository<P: PostRepository> {
    inner: P,
    cache: Arc<dyn CacheStore>,
    ttl: Duration,
}

impl<P: PostRepository> CachedPostRepository<P> {
    /// Create a cache decorator
    ///
    /// - `inner`: the decorated Repository
    /// - `cache`: cache storage backend
    /// - `ttl`: cache entry time-to-live (defaults to 5 minutes)
    pub fn new(inner: P, cache: Arc<dyn CacheStore>, ttl: Option<Duration>) -> Self {
        Self {
            inner,
            cache,
            ttl: ttl.unwrap_or(DEFAULT_TTL),
        }
    }

    /// Invalidate all post-related cache entries for the given tenant
    async fn invalidate_all(&self, tenant_id: Option<&str>) {
        let prefix = format!("{KEY_PREFIX}:{}:", tid(tenant_id));
        let _ = self.cache.delete_prefix(&prefix).await;
    }

    /// Invalidate cache entries related to the given ID (including slug index)
    async fn invalidate_by_id(&self, tenant_id: Option<&str>, id: i64) {
        let id_key = cache_key_id(tenant_id, id);
        if let Some(cached) = self.cache.get(&id_key).await
            && let Ok(post) = serde_json::from_str::<Post>(&cached)
        {
            let _ = self
                .cache
                .delete(&cache_key_slug(tenant_id, &post.slug))
                .await;
        }
        let _ = self.cache.delete(&id_key).await;
        let _ = self.cache.delete(&cache_key_joined(tenant_id, id)).await;
        let _ = self
            .cache
            .delete_prefix(&format!("{KEY_PREFIX}:{}:list:", tid(tenant_id)))
            .await;
    }
}

#[async_trait::async_trait]
impl<P: PostRepository> PostRepository for CachedPostRepository<P> {
    fn pool(&self) -> &crate::db::Pool {
        self.inner.pool()
    }

    async fn find_by_slug(&self, slug: &str, tenant_id: Option<&str>) -> AppResult<Option<Post>> {
        let key = cache_key_slug(tenant_id, slug);
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(post) = serde_json::from_str::<Post>(&cached)
        {
            return Ok(Some(post));
        }

        let result = self.inner.find_by_slug(slug, tenant_id).await?;
        if let Some(ref post) = result
            && let Ok(json) = serde_json::to_string(post)
        {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
            let id_key = cache_key_id(tenant_id, post.id);
            let _ = self.cache.set(&id_key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Post>> {
        let key = cache_key_id(tenant_id, id);
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(post) = serde_json::from_str::<Post>(&cached)
        {
            return Ok(Some(post));
        }

        let result = self.inner.find_by_id(id, tenant_id).await?;
        if let Some(ref post) = result
            && let Ok(json) = serde_json::to_string(post)
        {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
            let slug_key = cache_key_slug(tenant_id, &post.slug);
            let _ = self.cache.set(&slug_key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_joined_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<PostJoinedRow> {
        let key = cache_key_joined(tenant_id, id);
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(row) = serde_json::from_str::<PostJoinedRow>(&cached)
        {
            return Ok(row);
        }

        let result = self.inner.find_joined_by_id(id, tenant_id).await?;
        if let Ok(json) = serde_json::to_string(&result) {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_published_joined(
        &self,
        query: FindPublishedQuery,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)> {
        let key = format!(
            "{KEY_PREFIX}:{}:list:{}:{}:{}:{}:{}",
            tid(tenant_id),
            query.page,
            query.page_size,
            query
                .category_id
                .map(|id| id.to_string())
                .as_deref()
                .unwrap_or(""),
            query
                .tag_id
                .map(|id| id.to_string())
                .as_deref()
                .unwrap_or(""),
            query.q.as_deref().unwrap_or("")
        );
        if let Some(cached) = self.cache.get(&key).await
            && let Ok(result) = serde_json::from_str::<(Vec<PostJoinedRow>, i64)>(&cached)
        {
            return Ok(result);
        }

        let result = self.inner.find_published_joined(query, tenant_id).await?;
        if let Ok(json) = serde_json::to_string(&result) {
            let _ = self.cache.set(&key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_all_joined(
        &self,
        page: i64,
        page_size: i64,
        status: Option<PostStatus>,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)> {
        self.inner
            .find_all_joined(page, page_size, status, tenant_id)
            .await
    }

    async fn increment_view_count_joined(
        &self,
        slug: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<PostJoinedRow> {
        let result = self
            .inner
            .increment_view_count_joined(slug, tenant_id)
            .await?;
        self.invalidate_all(tenant_id).await;
        Ok(result)
    }

    async fn get_post_tags(
        &self,
        post_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<TagBrief>> {
        self.inner.get_post_tags(post_id, tenant_id).await
    }

    async fn get_tags_for_posts(
        &self,
        post_ids: &[i64],
        tenant_id: Option<&str>,
    ) -> AppResult<HashMap<i64, Vec<TagBrief>>> {
        self.inner.get_tags_for_posts(post_ids, tenant_id).await
    }

    async fn find_joined_by_ids(
        &self,
        ids: &[i64],
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PostJoinedRow>> {
        self.inner.find_joined_by_ids(ids, tenant_id).await
    }

    async fn create(&self, cmd: CreatePostCmd, tenant_id: Option<&str>) -> AppResult<Post> {
        let post = self.inner.create(cmd, tenant_id).await?;
        self.invalidate_all(tenant_id).await;
        Ok(post)
    }

    async fn update(&self, cmd: UpdatePostCmd, tenant_id: Option<&str>) -> AppResult<Post> {
        let post = self.inner.update(cmd, tenant_id).await?;
        self.invalidate_by_id(tenant_id, post.id).await;
        Ok(post)
    }

    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
        self.inner.delete(id, tenant_id).await?;
        self.invalidate_by_id(tenant_id, id).await;
        Ok(())
    }
}

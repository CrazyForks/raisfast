//! 缓存装饰器 — 为任意 `PostRepository` 添加缓存层
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

fn tid(tenant_id: Option<&str>) -> &str {
    tenant_id.unwrap_or("all")
}

fn cache_key_id(tenant_id: Option<&str>, id: &str) -> String {
    format!("{KEY_PREFIX}:{}:id:{id}", tid(tenant_id))
}

fn cache_key_slug(tenant_id: Option<&str>, slug: &str) -> String {
    format!("{KEY_PREFIX}:{}:slug:{slug}", tid(tenant_id))
}

fn cache_key_joined(tenant_id: Option<&str>, id: &str) -> String {
    format!("{KEY_PREFIX}:{}:joined:{id}", tid(tenant_id))
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

    /// 清除指定租户的所有文章相关缓存
    async fn invalidate_all(&self, tenant_id: Option<&str>) {
        let prefix = format!("{KEY_PREFIX}:{}:", tid(tenant_id));
        let _ = self.cache.delete_prefix(&prefix).await;
    }

    /// 清除指定 ID 相关的缓存（包括 slug 索引）
    async fn invalidate_by_id(&self, tenant_id: Option<&str>, id: &str) {
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
            let id_key = cache_key_id(tenant_id, &post.id);
            let _ = self.cache.set(&id_key, &json, Some(self.ttl)).await;
        }
        Ok(result)
    }

    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<Post>> {
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
        id: &str,
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
            query.category_id.as_deref().unwrap_or(""),
            query.tag_id.as_deref().unwrap_or(""),
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
        status: Option<&str>,
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
        post_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<TagBrief>> {
        self.inner.get_post_tags(post_id, tenant_id).await
    }

    async fn get_tags_for_posts(
        &self,
        post_ids: &[String],
        tenant_id: Option<&str>,
    ) -> AppResult<HashMap<String, Vec<TagBrief>>> {
        self.inner.get_tags_for_posts(post_ids, tenant_id).await
    }

    async fn find_joined_by_ids(
        &self,
        ids: &[String],
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
        self.invalidate_by_id(tenant_id, &post.id).await;
        Ok(post)
    }

    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
        self.inner.delete(id, tenant_id).await?;
        self.invalidate_by_id(tenant_id, id).await;
        Ok(())
    }
}

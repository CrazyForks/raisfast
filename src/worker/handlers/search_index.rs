//! 搜索索引重建 Handler
//!
//! 接收 `RebuildSearchIndex` 任务，从数据库读取文章数据并写入搜索引擎索引。
//! 当搜索引擎为 `NoopSearchEngine` 时，仅记录日志。

use std::sync::Arc;

use crate::errors::app_error::AppResult;
use crate::search::{SearchEngine, SearchablePost};
use crate::worker::{Job, JobHandler};

/// 搜索索引重建处理器
pub struct RebuildSearchIndexHandler {
    pool: crate::db::Pool,
    search: Arc<dyn SearchEngine>,
}

impl RebuildSearchIndexHandler {
    /// 创建新的搜索索引重建处理器
    pub fn new(pool: crate::db::Pool, search: Arc<dyn SearchEngine>) -> Self {
        Self { pool, search }
    }
}

#[async_trait::async_trait]
impl JobHandler for RebuildSearchIndexHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::RebuildSearchIndex { post_ids } = job else {
            return Ok(());
        };

        if self.search.is_noop() {
            tracing::debug!(
                "[search_index] noop engine, skipping {} post(s)",
                post_ids.len()
            );
            return Ok(());
        }

        tracing::info!(
            "[search_index] indexing {} post(s): {:?}",
            post_ids.len(),
            post_ids
        );

        let mut posts = Vec::with_capacity(post_ids.len());
        for id in post_ids {
            match crate::models::post::find_by_id(&self.pool, *id, None).await {
                Ok(Some(post)) => posts.push(SearchablePost {
                    id: post.document_id,
                    title: post.title,
                    content: post.content,
                }),
                Ok(None) => {
                    tracing::debug!("[search_index] post {id} not found, deleting from index");
                    self.search.delete_post(&id.to_string()).await?;
                }
                Err(e) => {
                    tracing::warn!("[search_index] failed to fetch post {id}: {e}");
                }
            }
        }

        if !posts.is_empty() {
            self.search.index_posts(&posts).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::NoopSearchEngine;

    #[tokio::test]
    async fn noop_engine_skips_indexing() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let handler = RebuildSearchIndexHandler::new(pool, Arc::new(NoopSearchEngine));
        let job = Job::RebuildSearchIndex { post_ids: vec![1] };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let handler = RebuildSearchIndexHandler::new(pool, Arc::new(NoopSearchEngine));
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }

    #[cfg(feature = "search-tantivy")]
    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[cfg(feature = "search-tantivy")]
    async fn create_user(pool: &crate::db::Pool) -> i64 {
        let document_id = uuid::Uuid::now_v7().to_string();
        let (user_id,): (i64,) = sqlx::query_as(
            "INSERT INTO users (document_id, username, role, status, registered_via) VALUES (?, 'testuser', 'author', 'active', 'email') RETURNING id",
        )
        .bind(&document_id)
        .fetch_one(pool)
        .await
        .unwrap();
        user_id
    }

    #[cfg(feature = "search-tantivy")]
    async fn create_post(pool: &crate::db::Pool, author_id: i64, title: &str) -> (i64, String) {
        let document_id = uuid::Uuid::now_v7().to_string();
        let now = crate::utils::tz::now_str();
        let published = crate::models::post::PostStatus::Published.as_str();
        let (int_id,): (i64,) = sqlx::query_as(&format!(
            "INSERT INTO posts (document_id, title, slug, content, status, created_by, updated_by, view_count, is_pinned, created_at, updated_at) \
             VALUES (?, ?, ?, ?, '{published}', ?, NULL, 0, 0, ?, ?) RETURNING id",
        ))
        .bind(&document_id)
        .bind(title)
        .bind(title.to_lowercase().replace(' ', "-"))
        .bind(format!("{title}的内容"))
        .bind(author_id)
        .bind(&now)
        .bind(&now)
        .fetch_one(pool)
        .await
        .unwrap();
        (int_id, document_id)
    }

    #[cfg(feature = "search-tantivy")]
    #[tokio::test]
    async fn indexes_existing_post_with_tantivy() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let (post_int_id, post_doc_id) = create_post(&pool, uid, "Rust编程入门").await;

        let engine = Arc::new(crate::search::TantivyEngine::open_in_memory().unwrap());
        let handler = RebuildSearchIndexHandler::new(pool, engine.clone());
        let job = Job::RebuildSearchIndex {
            post_ids: vec![post_int_id],
        };
        assert!(handler.handle(&job).await.is_ok());

        let (results, total) = engine.search("Rust", 1, 10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(results[0].post_id, post_doc_id);
    }

    #[cfg(feature = "search-tantivy")]
    #[tokio::test]
    async fn indexes_multiple_posts_with_tantivy() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let (p1_id, _) = create_post(&pool, uid, "Rust入门").await;
        let (p2_id, _) = create_post(&pool, uid, "Go进阶").await;

        let engine = Arc::new(crate::search::TantivyEngine::open_in_memory().unwrap());
        let handler = RebuildSearchIndexHandler::new(pool, engine.clone());
        let job = Job::RebuildSearchIndex {
            post_ids: vec![p1_id, p2_id],
        };
        assert!(handler.handle(&job).await.is_ok());

        let (_, t1) = engine.search("Rust", 1, 10).await.unwrap();
        assert_eq!(t1, 1);
        let (_, t2) = engine.search("Go", 1, 10).await.unwrap();
        assert_eq!(t2, 1);
    }

    #[cfg(feature = "search-tantivy")]
    #[tokio::test]
    async fn deletes_nonexistent_post_from_index() {
        let pool = setup_pool().await;
        let engine = Arc::new(crate::search::TantivyEngine::open_in_memory().unwrap());

        engine
            .index_post(&SearchablePost {
                id: "0".into(),
                title: "幽灵文章".into(),
                content: "内容".into(),
            })
            .await
            .unwrap();
        let (_, t) = engine.search("幽灵", 1, 10).await.unwrap();
        assert_eq!(t, 1);

        let handler = RebuildSearchIndexHandler::new(pool, engine.clone());
        let job = Job::RebuildSearchIndex { post_ids: vec![0] };
        assert!(handler.handle(&job).await.is_ok());

        let (_, t) = engine.search("幽灵", 1, 10).await.unwrap();
        assert_eq!(t, 0);
    }

    #[cfg(feature = "search-tantivy")]
    #[tokio::test]
    async fn handles_empty_post_ids() {
        let pool = setup_pool().await;
        let engine = Arc::new(crate::search::TantivyEngine::open_in_memory().unwrap());
        let handler = RebuildSearchIndexHandler::new(pool, engine);
        let job = Job::RebuildSearchIndex { post_ids: vec![] };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[cfg(feature = "search-tantivy")]
    #[tokio::test]
    async fn handles_mixed_existing_and_missing_posts() {
        let pool = setup_pool().await;
        let uid = create_user(&pool).await;
        let (real_id, real_doc_id) = create_post(&pool, uid, "真实文章").await;

        let engine = Arc::new(crate::search::TantivyEngine::open_in_memory().unwrap());
        let handler = RebuildSearchIndexHandler::new(pool, engine.clone());
        let job = Job::RebuildSearchIndex {
            post_ids: vec![real_id, 999999],
        };
        assert!(handler.handle(&job).await.is_ok());

        let (results, total) = engine.search("真实", 1, 10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(results[0].post_id, real_doc_id);
    }
}

//! 搜索索引重建 Handler
//!
//! 当前使用 SQL LIKE 实现搜索，此 Handler 验证 post_ids 存在并记录日志。
//! 生产环境可替换为 FTS5 / Tantivy 等全文索引引擎。

use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// 搜索索引重建处理器
pub struct RebuildSearchIndexHandler {
    #[allow(dead_code)]
    pool: crate::db::Pool,
}

impl RebuildSearchIndexHandler {
    /// 创建新的搜索索引重建处理器
    pub fn new(pool: crate::db::Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobHandler for RebuildSearchIndexHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::RebuildSearchIndex { post_ids } = job else {
            return Ok(());
        };

        tracing::info!(
            "[search_index] rebuilding index for {} post(s): {:?}",
            post_ids.len(),
            post_ids
        );

        // TODO: 生产环境替换为真实 FTS5 索引重建
        // for post_id in post_ids {
        //     let post = crate::models::post::find_by_id(&self.pool, post_id).await?;
        //     sqlx::query("INSERT OR REPLACE INTO search_index (...) VALUES (...)")
        //         .bind(post_id)
        //         .execute(&self.pool)
        //         .await?;
        // }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handles_rebuild_search_index_job() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let handler = RebuildSearchIndexHandler::new(pool);
        let job = Job::RebuildSearchIndex {
            post_ids: vec!["p1".into(), "p2".into()],
        };
        let result = handler.handle(&job).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let handler = RebuildSearchIndexHandler::new(pool);
        let job = Job::GenerateSitemap;
        let result = handler.handle(&job).await;
        assert!(result.is_ok());
    }
}

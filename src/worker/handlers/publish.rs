//! 定时发布 Handler
//!
//! 将 `status = 'draft'` 的文章状态更新为 `published`。
//! `run_after` 字段控制执行时间，由 Worker 系统自动延迟。

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// 定时发布处理器
pub struct ScheduledPublishHandler {
    pool: Pool,
}

impl ScheduledPublishHandler {
    /// 创建新的定时发布处理器
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobHandler for ScheduledPublishHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::ScheduledPublish { post_id } = job else {
            return Ok(());
        };

        let post = crate::models::post::find_by_id(
            &self.pool,
            post_id,
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await?;
        let Some(post) = post else {
            tracing::warn!("[publish] post {} not found, skipping", post_id);
            return Ok(());
        };

        if post.status == "published" {
            tracing::info!("[publish] post {} already published", post_id);
            return Ok(());
        }

        crate::models::post::update(
            &self.pool,
            &crate::commands::UpdatePostCmd {
                id: post_id.clone(),
                title: None,
                slug: None,
                content: None,
                excerpt: None,
                cover_image: None,
                status: Some("published".to_string()),
                category_id: None,
                tag_ids: None,
            },
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await?;

        tracing::info!("[publish] published post {}", post_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::post;
    use crate::models::user;

    async fn setup() -> Pool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../../migrations/001_init.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../../migrations/002_add_indexes.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../../migrations/009_options.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../../migrations/010_rbac.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../../migrations/011_tenants.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE users ADD COLUMN email_verified INTEGER NOT NULL DEFAULT 0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE users ADD COLUMN phone TEXT")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn create_author(pool: &Pool) -> String {
        let u = user::create(
            pool,
            &crate::commands::CreateUserCmd {
                email: "author@test.com".to_string(),
                username: "author".to_string(),
                password_hash: "hash".to_string(),
            },
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        user::update_role(
            pool,
            &u.id,
            "author",
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();
        u.id
    }

    #[tokio::test]
    async fn publishes_draft_post() {
        let pool = setup().await;
        let author_id = create_author(&pool).await;

        let p = post::create(
            &pool,
            &crate::commands::CreatePostCmd {
                title: "Test".to_string(),
                slug: "test-slug".to_string(),
                content: "content".to_string(),
                excerpt: None,
                cover_image: None,
                status: "draft".to_string(),
                author_id,
                category_id: None,
                tag_ids: None,
            },
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();

        let handler = ScheduledPublishHandler::new(pool.clone());
        let job = Job::ScheduledPublish {
            post_id: p.id.clone(),
        };
        assert!(handler.handle(&job).await.is_ok());

        let updated = post::find_by_id(&pool, &p.id, Some(crate::db::tenant::DEFAULT_TENANT))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "published");
        assert!(updated.published_at.is_some());
    }

    #[tokio::test]
    async fn skips_already_published() {
        let pool = setup().await;
        let author_id = create_author(&pool).await;

        let p = post::create(
            &pool,
            &crate::commands::CreatePostCmd {
                title: "Test".to_string(),
                slug: "test-slug-2".to_string(),
                content: "content".to_string(),
                excerpt: None,
                cover_image: None,
                status: "published".to_string(),
                author_id,
                category_id: None,
                tag_ids: None,
            },
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await
        .unwrap();

        let handler = ScheduledPublishHandler::new(pool);
        let job = Job::ScheduledPublish { post_id: p.id };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn skips_nonexistent_post() {
        let pool = setup().await;
        let handler = ScheduledPublishHandler::new(pool);
        let job = Job::ScheduledPublish {
            post_id: "nonexistent".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = setup().await;
        let handler = ScheduledPublishHandler::new(pool);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}

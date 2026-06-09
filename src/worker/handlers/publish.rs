//! Scheduled publish Handler
//!
//! Updates post status from `status = 'draft'` to `published`.
//! The `run_after` field controls execution timing, handled automatically by the Worker system.

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::post::PostStatus;
use crate::worker::{Job, JobHandler};

/// Scheduled publish handler
pub struct ScheduledPublishHandler {
    pool: Pool,
}

impl ScheduledPublishHandler {
    /// Creates a new scheduled publish handler
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

        let post = crate::models::post::find_by_id(&self.pool, *post_id, None).await?;
        let Some(post) = post else {
            tracing::warn!("[publish] post {} not found, skipping", post_id);
            return Ok(());
        };

        if post.status == PostStatus::Published {
            tracing::info!("[publish] post {} already published", post_id);
            return Ok(());
        }

        crate::models::post::update(
            &self.pool,
            &crate::commands::UpdatePostCmd {
                id: post.id,
                title: None,
                slug: None,
                content: None,
                excerpt: None,
                cover_image: None,
                image_ids: None,
                status: Some(crate::models::post::PostStatus::Published),
                category_id: None,
                tag_ids: None,
                updated_by: None,
                meta_title: None,
                meta_description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
            },
            None,
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
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup() -> Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn create_author(pool: &Pool) -> i64 {
        let u = user::create(
            pool,
            &crate::commands::CreateUserCmd {
                username: "author".to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
            },
            None,
        )
        .await
        .unwrap();
        user::update_role(pool, u.id, crate::models::user::UserRole::Author, None)
            .await
            .unwrap();
        *u.id
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
                image_ids: None,
                status: crate::models::post::PostStatus::Draft,
                created_by: author_id,
                updated_by: None,
                category_id: None,
                tag_ids: None,
                meta_title: None,
                meta_description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
            },
            None,
        )
        .await
        .unwrap();

        let handler = ScheduledPublishHandler::new(pool.clone());
        let job = Job::ScheduledPublish { post_id: p.id };
        assert!(handler.handle(&job).await.is_ok());

        let updated = post::find_by_id(&pool, p.id, None).await.unwrap().unwrap();
        assert_eq!(updated.status, PostStatus::Published);
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
                image_ids: None,
                status: crate::models::post::PostStatus::Published,
                created_by: author_id,
                updated_by: None,
                category_id: None,
                tag_ids: None,
                meta_title: None,
                meta_description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
            },
            None,
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
            post_id: SnowflakeId(999999),
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

//! `EventBus` → `JobQueue` bridge
//!
//! Subscribes to `EventBus` events and automatically creates corresponding async jobs.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::eventbus::{Event, EventBus};

use super::{Job, JobQueue, NewJob};

/// Bridges `EventBus` events to jobs
pub struct JobEnqueuer {
    queue: Arc<dyn JobQueue>,
}

impl JobEnqueuer {
    /// Spawns a background subscriber
    pub fn spawn(eventbus: &EventBus, queue: Arc<dyn JobQueue>) {
        let mut rx = eventbus.subscribe();
        let enqueuer = Self { queue };

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => enqueuer.on_event(&event).await,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("job enqueuer lagged, skipped {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            tracing::error!("job enqueuer exited unexpectedly");
        });
    }

    async fn on_event(&self, event: &Event) {
        let jobs = self.create_jobs(event);
        for new_job in jobs {
            let job_type = new_job.job.job_type().to_string();
            if let Err(e) = self.queue.enqueue(new_job).await {
                tracing::error!("failed to enqueue {job_type}: {e}");
            }
        }
    }

    fn create_jobs(&self, event: &Event) -> Vec<NewJob> {
        match event {
            Event::PostCreated(data) => {
                let post_id: i64 = data.id.parse().unwrap_or(0);
                vec![NewJob::from(Job::RebuildSearchIndex {
                    post_ids: vec![post_id],
                })]
            }
            Event::PostUpdated(data) => {
                let post_id: i64 = *data.id;
                vec![NewJob::from(Job::RebuildSearchIndex {
                    post_ids: vec![post_id],
                })]
            }
            Event::PostDeleted(data) => {
                let post_id: i64 = *data.id;
                vec![NewJob::from(Job::RebuildSearchIndex {
                    post_ids: vec![post_id],
                })]
            }
            Event::UserRegistered(data) => {
                vec![NewJob::from(Job::SendWelcomeEmail {
                    user_id: data.id,
                    email: String::new(),
                    username: data.username.clone(),
                })]
            }
            Event::MediaUploaded(data) => {
                vec![NewJob::from(Job::GenerateThumbnail {
                    media_id: data.id,
                    size: 300,
                })]
            }
            Event::PasswordResetRequested { user, token } => {
                vec![NewJob::from(Job::SendPasswordResetEmail {
                    user_id: user.id,
                    email: String::new(),
                    reset_token: token.token.clone(),
                })]
            }
            Event::EmailVerificationRequested {
                user_id,
                email,
                token,
            } => {
                vec![NewJob::from(Job::SendEmailVerification {
                    user_id: *user_id,
                    email: email.clone(),
                    verify_token: token.token.clone(),
                })]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::PostResponse;
    use crate::models::comment::{Comment, CommentStatus};
    use crate::models::media::Media;
    use crate::models::post::{CommentOpenStatus, PostStatus};
    use crate::models::user::{RegisteredVia, User, UserRole, UserStatus};
    use crate::worker::DefaultJobQueue;

    fn ts() -> crate::utils::tz::Timestamp {
        "2025-01-01T00:00:00Z".parse().unwrap()
    }

    fn make_post_response(id: &str, slug: &str, title: &str) -> PostResponse {
        PostResponse {
            id: id.into(),
            title: title.into(),
            slug: slug.into(),
            content: String::new(),
            excerpt: None,
            cover_image: None,
            image_ids: None,
            status: PostStatus::Published,
            created_by: None,
            author_name: None,
            category_id: None,
            category_name: None,
            tags: vec![],
            view_count: 0,
            is_pinned: false,
            password: None,
            comment_status: CommentOpenStatus::Open,
            format: String::new(),
            template: String::new(),
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
            canonical_url: None,
            reading_time: 0,
            created_at: ts(),
            updated_at: ts(),
            published_at: None,
            title_highlight: None,
            excerpt_highlight: None,
        }
    }

    fn make_user(id: i64, username: &str) -> User {
        User {
            id: crate::types::snowflake_id::SnowflakeId(id),
            tenant_id: None,
            username: username.into(),
            role: UserRole::Reader,
            status: UserStatus::Active,
            registered_via: RegisteredVia::Email,
            avatar: None,
            bio: None,
            website: None,
            display_name: None,
            slug: None,
            locale: None,
            social_links: None,
            metadata: None,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn make_media(id: i64, filename: &str) -> Media {
        Media {
            id: crate::types::snowflake_id::SnowflakeId(id),
            tenant_id: None,
            user_id: crate::types::snowflake_id::SnowflakeId(1),
            filename: filename.into(),
            filepath: String::new(),
            mimetype: String::new(),
            size: 0,
            width: None,
            height: None,
            title: None,
            alt_text: None,
            caption: None,
            description: None,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn make_comment(id: i64) -> Comment {
        Comment {
            id: crate::types::snowflake_id::SnowflakeId(id),
            tenant_id: None,
            post_id: crate::types::snowflake_id::SnowflakeId(0),
            created_by: None,
            updated_by: None,
            nickname: None,
            email: None,
            content: String::new(),
            parent_id: None,
            author_ip: None,
            author_url: None,
            status: CommentStatus::Approved,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    async fn setup() -> (EventBus, Arc<DefaultJobQueue>) {
        let bus = EventBus::new(16);
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let queue = Arc::new(DefaultJobQueue::new(pool));
        (bus, queue)
    }

    #[tokio::test]
    async fn post_created_enqueues_rebuild_search_index() {
        let (bus, queue) = setup().await;
        JobEnqueuer::spawn(&bus, queue.clone());

        bus.emit(Event::PostCreated(make_post_response(
            "1", "hello", "Hello",
        )));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(matches!(
            &jobs[0].job,
            Job::RebuildSearchIndex { post_ids } if post_ids == &vec![1]
        ));
    }

    #[tokio::test]
    async fn user_registered_enqueues_send_welcome_email() {
        let (bus, queue) = setup().await;
        JobEnqueuer::spawn(&bus, queue.clone());

        bus.emit(Event::UserRegistered(make_user(1, "alice")));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(matches!(
            &jobs[0].job,
            Job::SendWelcomeEmail { user_id, email, username }
            if user_id == &1 && email.is_empty() && username == "alice"
        ));
    }

    #[tokio::test]
    async fn media_uploaded_enqueues_generate_thumbnail() {
        let (bus, queue) = setup().await;
        JobEnqueuer::spawn(&bus, queue.clone());

        bus.emit(Event::MediaUploaded(make_media(1, "photo.jpg")));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(matches!(
            &jobs[0].job,
            Job::GenerateThumbnail { media_id, size: 300 } if media_id == &1
        ));
    }

    #[tokio::test]
    async fn untracked_events_enqueue_nothing() {
        let (bus, queue) = setup().await;
        JobEnqueuer::spawn(&bus, queue.clone());

        bus.emit(Event::CommentCreated(make_comment(1)));
        bus.emit(Event::UserLoggedIn {
            user: make_user(1, "u"),
            success: true,
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let jobs = queue.dequeue(10).await.unwrap();
        assert!(jobs.is_empty());
    }

    #[tokio::test]
    async fn multiple_events_enqueue_multiple_jobs() {
        let (bus, queue) = setup().await;
        JobEnqueuer::spawn(&bus, queue.clone());

        bus.emit(Event::PostCreated(make_post_response("1", "a", "A")));
        bus.emit(Event::PostCreated(make_post_response("2", "b", "B")));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 2);
    }
}

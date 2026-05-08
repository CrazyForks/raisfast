//! `EventBus` → `JobQueue` 桥接
//!
//! 订阅 `EventBus` 事件，自动创建对应的异步任务。

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::eventbus::{Event, EventBus};

use super::{Job, JobQueue, NewJob};

/// `EventBus` 事件到任务的桥接器
pub struct JobEnqueuer {
    queue: Arc<dyn JobQueue>,
}

impl JobEnqueuer {
    /// 启动后台订阅者
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
            Event::PostCreated { id, .. } => {
                let post_id: i64 = id.parse().unwrap_or(0);
                vec![NewJob::from(Job::RebuildSearchIndex {
                    post_ids: vec![post_id],
                })]
            }
            Event::PostUpdated { id, .. } => {
                let post_id: i64 = id.parse().unwrap_or(0);
                vec![NewJob::from(Job::RebuildSearchIndex {
                    post_ids: vec![post_id],
                })]
            }
            Event::PostDeleted { id, .. } => {
                let post_id: i64 = id.parse().unwrap_or(0);
                vec![NewJob::from(Job::RebuildSearchIndex {
                    post_ids: vec![post_id],
                })]
            }
            Event::UserRegistered {
                id,
                email,
                username,
            } => {
                let user_id: i64 = id.parse().unwrap_or(0);
                vec![NewJob::from(Job::SendWelcomeEmail {
                    user_id,
                    email: email.clone(),
                    username: username.clone(),
                })]
            }
            Event::MediaUploaded { id, .. } => {
                let media_id: i64 = id.parse().unwrap_or(0);
                vec![NewJob::from(Job::GenerateThumbnail {
                    media_id,
                    size: 300,
                })]
            }
            Event::PasswordResetRequested {
                user_id,
                email,
                reset_token,
            } => {
                let uid: i64 = user_id.parse().unwrap_or(0);
                vec![NewJob::from(Job::SendPasswordResetEmail {
                    user_id: uid,
                    email: email.clone(),
                    reset_token: reset_token.clone(),
                })]
            }
            Event::EmailVerificationRequested {
                user_id,
                email,
                verify_token,
            } => {
                let uid: i64 = user_id.parse().unwrap_or(0);
                vec![NewJob::from(Job::SendEmailVerification {
                    user_id: uid,
                    email: email.clone(),
                    verify_token: verify_token.clone(),
                })]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::SqliteJobQueue;

    async fn setup() -> (EventBus, Arc<SqliteJobQueue>) {
        let bus = EventBus::new(16);
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let queue = Arc::new(SqliteJobQueue::new(pool));
        (bus, queue)
    }

    #[tokio::test]
    async fn post_created_enqueues_rebuild_search_index() {
        let (bus, queue) = setup().await;
        JobEnqueuer::spawn(&bus, queue.clone());

        bus.emit(Event::PostCreated {
            id: "1".into(),
            slug: "hello".into(),
            title: "Hello".into(),
            author_id: "u1".into(),
        });

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

        bus.emit(Event::UserRegistered {
            id: "1".into(),
            username: "alice".into(),
            email: "alice@example.com".into(),
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(matches!(
            &jobs[0].job,
            Job::SendWelcomeEmail { user_id, email, username }
            if user_id == &1 && email == "alice@example.com" && username == "alice"
        ));
    }

    #[tokio::test]
    async fn media_uploaded_enqueues_generate_thumbnail() {
        let (bus, queue) = setup().await;
        JobEnqueuer::spawn(&bus, queue.clone());

        bus.emit(Event::MediaUploaded {
            id: "1".into(),
            filename: "photo.jpg".into(),
            uploader_id: "1".into(),
        });

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

        bus.emit(Event::CommentCreated {
            id: "1".into(),
            post_slug: "hello".into(),
            author_name: "bob".into(),
        });
        bus.emit(Event::UserLoggedIn {
            id: "1".into(),
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

        bus.emit(Event::PostCreated {
            id: "1".into(),
            slug: "a".into(),
            title: "A".into(),
            author_id: "u1".into(),
        });
        bus.emit(Event::PostCreated {
            id: "2".into(),
            slug: "b".into(),
            title: "B".into(),
            author_id: "u1".into(),
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 2);
    }
}

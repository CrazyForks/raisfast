//! Visibility timeout sweeper — reclaims jobs stuck in `running` state.
//!
//! When a worker crashes or panics mid-execution, the job remains in `running`
//! forever. This sweeper periodically scans for such jobs and either requeues
//! them (if retries remain) or marks them dead (if `max_attempts` exhausted).

use std::sync::Arc;
use std::time::Duration;

use super::JobQueue;

/// Background sweeper that periodically requeues jobs stuck in `running`.
pub struct StuckJobSweeper {
    queue: Arc<dyn JobQueue>,
    sweep_interval: Duration,
    visibility_timeout: Duration,
}

impl StuckJobSweeper {
    /// Creates a new sweeper.
    ///
    /// # Parameters
    /// - `queue` — the shared job queue
    /// - `sweep_interval` — how often to scan (e.g. 60s)
    /// - `visibility_timeout` — how long a job may stay `running` before reclaim
    #[must_use]
    pub fn new(
        queue: Arc<dyn JobQueue>,
        sweep_interval: Duration,
        visibility_timeout: Duration,
    ) -> Self {
        Self {
            queue,
            sweep_interval,
            visibility_timeout,
        }
    }

    /// Spawns the background sweep loop.
    pub fn spawn(self) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.sweep_interval);
            // Don't sweep immediately on startup — give workers a grace period.
            interval.tick().await;

            loop {
                interval.tick().await;
                if let Err(e) = self.queue.requeue_stuck(self.visibility_timeout).await {
                    tracing::error!("stuck-job sweeper error: {e}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{DefaultJobQueue, Job, NewJob};

    async fn setup() -> Arc<DefaultJobQueue> {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        Arc::new(DefaultJobQueue::new(pool))
    }

    #[tokio::test]
    async fn requeue_stuck_returns_zero_on_empty() {
        let q = setup().await;
        let count = q.requeue_stuck(Duration::from_secs(1)).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn requeue_stuck_requeues_running_job() {
        let q = setup().await;
        q.enqueue(NewJob::from(Job::GenerateSitemap)).await.unwrap();
        let _ = q.dequeue(10).await.unwrap(); // status → running

        // Use 0s timeout so everything is immediately "stuck"
        let count = q.requeue_stuck(Duration::from_secs(0)).await.unwrap();
        assert_eq!(count, 1);

        // Should be dequeueable again
        let jobs = q.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].attempts, 2); // dequeued twice
    }

    #[tokio::test]
    async fn requeue_stuck_marks_dead_after_max_attempts() {
        let q = setup().await;
        q.enqueue(NewJob {
            job: Job::GenerateSitemap,
            max_attempts: Some(1),
            run_after: None,
        })
        .await
        .unwrap();
        let _ = q.dequeue(10).await.unwrap(); // attempts 0→1, running

        let count = q.requeue_stuck(Duration::from_secs(0)).await.unwrap();
        assert_eq!(count, 1);

        // Should NOT be dequeueable — it's dead
        let jobs = q.dequeue(10).await.unwrap();
        assert!(jobs.is_empty());
    }
}

//! Worker background polling executor
//!
//! Dispatch chain: built-in Handler Registry → plugin Cron Dispatcher → mark dead

use std::sync::Arc;
use std::time::Duration;

use super::{Job, JobHandlerRegistry, JobQueue, PluginCronDispatcher};

/// Worker executor
pub struct WorkerRunner {
    queue: Arc<dyn JobQueue>,
    handlers: Arc<JobHandlerRegistry>,
    plugin_dispatcher: Option<Arc<PluginCronDispatcher>>,
    poll_interval: Duration,
    batch_size: usize,
}

impl WorkerRunner {
    /// Creates a new `WorkerRunner`
    ///
    /// When `plugin_dispatcher` is `None`, unmatched jobs are directly marked dead.
    pub fn new(
        queue: Arc<dyn JobQueue>,
        handlers: Arc<JobHandlerRegistry>,
        poll_interval: Duration,
        batch_size: usize,
    ) -> Self {
        Self {
            queue,
            handlers,
            plugin_dispatcher: None,
            poll_interval,
            batch_size,
        }
    }

    /// Sets the plugin Cron dispatcher
    #[must_use]
    pub fn with_plugin_dispatcher(mut self, dispatcher: Arc<PluginCronDispatcher>) -> Self {
        self.plugin_dispatcher = Some(dispatcher);
        self
    }

    /// Spawns N concurrent workers
    pub fn spawn(self, concurrency: usize) {
        for i in 0..concurrency {
            let runner = self.clone_for_worker();
            tokio::spawn(async move {
                tracing::info!("worker-{i} started");
                runner.run(i).await;
                tracing::error!("worker-{i} exited unexpectedly");
            });
        }
    }

    async fn run(self, worker_id: usize) {
        let mut interval = tokio::time::interval(self.poll_interval);

        loop {
            interval.tick().await;

            match self.queue.dequeue(self.batch_size).await {
                Ok(jobs) => {
                    self.execute_batch(&jobs, worker_id).await;
                }
                Err(e) => {
                    tracing::error!("worker-{worker_id} dequeue error: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn execute_batch(&self, jobs: &[super::QueuedJob], worker_id: usize) {
        let mut search_ids: Vec<i64> = Vec::new();
        let mut search_job_ids: Vec<String> = Vec::new();

        for job in jobs {
            if let Job::RebuildSearchIndex { post_ids } = &job.job {
                search_ids.extend_from_slice(post_ids);
                search_job_ids.push(job.id.clone());
            } else {
                if let Err(e) = self.execute(job).await {
                    tracing::error!("worker-{worker_id} job {} error: {e}", job.id);
                }
            }
        }

        if !search_ids.is_empty() {
            search_ids.sort_unstable();
            search_ids.dedup();

            let merged_job = Job::RebuildSearchIndex {
                post_ids: search_ids,
            };
            let job_type = merged_job.job_type();

            let result = if self.handlers.has_handler(job_type) {
                self.handlers.handle(&merged_job).await
            } else {
                tracing::warn!("no handler for coalesced search index job");
                Ok(())
            };

            if let Err(e) = result {
                tracing::error!("worker-{worker_id} coalesced search index error: {e}");
                for id in &search_job_ids {
                    if let Err(e) = self.queue.fail(id, &format!("{e}")).await {
                        tracing::error!(
                            "worker-{worker_id} failed to fail coalesced job {id}: {e}"
                        );
                    }
                }
            } else {
                for id in &search_job_ids {
                    if let Err(e) = self.queue.complete(id).await {
                        tracing::error!(
                            "worker-{worker_id} failed to complete coalesced job {id}: {e}"
                        );
                    }
                }
            }
        }
    }

    async fn execute(&self, job: &super::QueuedJob) -> super::AppResult<()> {
        let job_type = job.job.job_type();

        tracing::debug!(
            "executing job {} type={} attempt={}/{}",
            job.id,
            job_type,
            job.attempts,
            job.max_attempts,
        );

        let result = if self.handlers.has_handler(job_type) {
            self.handlers.handle(&job.job).await
        } else if let Some(ref dispatcher) = self.plugin_dispatcher {
            tracing::info!("no built-in handler for '{job_type}', dispatching to plugins");
            dispatcher.dispatch(&job.job).await
        } else {
            tracing::warn!("no handler for job type '{job_type}', marking dead");
            self.queue.dead(&job.id, "no handler registered").await?;
            return Ok(());
        };

        match result {
            Ok(()) => self.queue.complete(&job.id).await,
            Err(e) => {
                let err_msg = format!("{e}");
                if job.attempts >= job.max_attempts {
                    self.queue.dead(&job.id, &err_msg).await
                } else {
                    self.queue.fail(&job.id, &err_msg).await
                }
            }
        }
    }

    fn clone_for_worker(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            handlers: self.handlers.clone(),
            plugin_dispatcher: self.plugin_dispatcher.clone(),
            poll_interval: self.poll_interval,
            batch_size: self.batch_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;
    use crate::worker::{DefaultJobQueue, Job, LogJobHandler, NewJob};

    struct FailHandler;

    #[async_trait::async_trait]
    impl crate::worker::JobHandler for FailHandler {
        async fn handle(&self, _job: &Job) -> crate::errors::app_error::AppResult<()> {
            Err(crate::errors::app_error::AppError::BadRequest(
                "fail".into(),
            ))
        }
    }

    async fn setup() -> (Arc<DefaultJobQueue>, Arc<JobHandlerRegistry>) {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let queue = Arc::new(DefaultJobQueue::new(pool));
        let mut registry = JobHandlerRegistry::new();
        registry.register("generate_sitemap", Box::new(LogJobHandler));
        registry.register("send_welcome_email", Box::new(FailHandler));
        registry.register("rebuild_search_index", Box::new(LogJobHandler));
        (queue, Arc::new(registry))
    }

    #[tokio::test]
    async fn execute_completes_on_handler_success() {
        let (queue, registry) = setup().await;
        let runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(100), 5);

        queue
            .enqueue(NewJob::from(Job::GenerateSitemap))
            .await
            .unwrap();
        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);

        let result = runner.execute(&jobs[0]).await;
        assert!(result.is_ok());

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.running, 0);
    }

    #[tokio::test]
    async fn execute_fails_and_retries() {
        let (queue, registry) = setup().await;
        let runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(100), 5);

        queue
            .enqueue(NewJob {
                job: Job::SendWelcomeEmail {
                    user_id: SnowflakeId(1),
                    email: "a@b.com".into(),
                    username: "alice".into(),
                },
                max_attempts: Some(3),
                run_after: None,
            })
            .await
            .unwrap();

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs[0].attempts, 1);

        let result = runner.execute(&jobs[0]).await;
        assert!(result.is_ok());

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.pending, 1);
    }

    #[tokio::test]
    async fn execute_marks_dead_at_max_attempts() {
        let (queue, registry) = setup().await;
        let runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(100), 5);

        queue
            .enqueue(NewJob {
                job: Job::SendWelcomeEmail {
                    user_id: SnowflakeId(1),
                    email: "a@b.com".into(),
                    username: "alice".into(),
                },
                max_attempts: Some(1),
                run_after: None,
            })
            .await
            .unwrap();

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs[0].attempts, 1);
        assert_eq!(jobs[0].max_attempts, 1);

        let result = runner.execute(&jobs[0]).await;
        assert!(result.is_ok());

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.dead, 1);
        assert_eq!(stats.pending, 0);
    }

    #[tokio::test]
    async fn dequeue_empty_no_error() {
        let (queue, registry) = setup().await;
        let _runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(100), 5);

        let jobs = queue.dequeue(10).await.unwrap();
        assert!(jobs.is_empty());

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.pending, 0);
    }

    #[tokio::test]
    async fn spawn_processes_pending_jobs() {
        let (queue, registry) = setup().await;
        let runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(50), 5);

        queue
            .enqueue(NewJob::from(Job::GenerateSitemap))
            .await
            .unwrap();

        runner.spawn(1);

        tokio::time::sleep(Duration::from_millis(300)).await;

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.completed, 1);
    }

    #[tokio::test]
    async fn unhandled_job_without_plugin_marks_dead() {
        let (queue, registry) = setup().await;
        let runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(100), 5);

        queue
            .enqueue(NewJob::from(Job::Custom {
                job_type: "unknown_task".into(),
                payload: serde_json::json!({"x": 1}),
            }))
            .await
            .unwrap();

        let jobs = queue.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);

        let result = runner.execute(&jobs[0]).await;
        assert!(result.is_ok());

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.dead, 1);
        assert_eq!(stats.completed, 0);
    }

    #[tokio::test]
    async fn coalesces_multiple_search_index_jobs() {
        let (queue, registry) = setup().await;
        let runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(100), 20);

        queue
            .enqueue(NewJob::from(Job::RebuildSearchIndex {
                post_ids: vec![1, 2],
            }))
            .await
            .unwrap();
        queue
            .enqueue(NewJob::from(Job::RebuildSearchIndex {
                post_ids: vec![2, 3],
            }))
            .await
            .unwrap();
        queue
            .enqueue(NewJob::from(Job::RebuildSearchIndex { post_ids: vec![4] }))
            .await
            .unwrap();

        let jobs = queue.dequeue(20).await.unwrap();
        assert_eq!(jobs.len(), 3);

        runner.execute_batch(&jobs, 0).await;

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.completed, 3);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.running, 0);
        assert_eq!(stats.dead, 0);
    }

    #[tokio::test]
    async fn coalesces_search_index_with_mixed_jobs() {
        let (queue, registry) = setup().await;
        let runner = WorkerRunner::new(queue.clone(), registry, Duration::from_millis(100), 20);

        queue
            .enqueue(NewJob::from(Job::GenerateSitemap))
            .await
            .unwrap();
        queue
            .enqueue(NewJob::from(Job::RebuildSearchIndex { post_ids: vec![10] }))
            .await
            .unwrap();
        queue
            .enqueue(NewJob::from(Job::RebuildSearchIndex { post_ids: vec![20] }))
            .await
            .unwrap();

        let jobs = queue.dequeue(20).await.unwrap();
        assert_eq!(jobs.len(), 3);

        runner.execute_batch(&jobs, 0).await;

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.completed, 3);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.dead, 0);
    }
}

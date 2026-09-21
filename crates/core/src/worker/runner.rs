//! Worker background polling executor
//!
//! Dispatch chain: built-in Handler Registry → plugin Cron Dispatcher → mark dead

use std::sync::Arc;
use std::time::Duration;

use crate::constants::COL_ID;
use crate::db::{DbDriver, Driver, Pool};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;

use super::{
    CronExecStatus, JobHandlerRegistry, JobQueue, JobStatus, JobTypeFilter, PluginCronDispatcher,
    QueuedJob,
};

/// Worker executor
pub struct WorkerRunner {
    queue: Arc<dyn JobQueue>,
    handlers: Arc<JobHandlerRegistry>,
    plugin_dispatcher: Option<Arc<PluginCronDispatcher>>,
    pool: Pool,
    poll_interval: Duration,
    batch_size: usize,
    /// Global job visibility timeout, used as the heartbeat cadence basis for
    /// jobs without a per-job `timeout_secs`. Mirrors
    /// `worker_visibility_timeout_secs`.
    visibility_timeout: Duration,
    /// Which job types this pool may claim. `Any` for a single-pool setup.
    claim: JobTypeFilter,
    /// Shutdown signal. When it flips true the worker stops claiming and exits
    /// after finishing the job it is currently running.
    shutdown: Option<tokio::sync::watch::Receiver<bool>>,
}

impl WorkerRunner {
    /// Creates a new `WorkerRunner`
    ///
    /// When `plugin_dispatcher` is `None`, unmatched jobs are directly marked dead.
    pub fn new(
        queue: Arc<dyn JobQueue>,
        handlers: Arc<JobHandlerRegistry>,
        pool: Pool,
        poll_interval: Duration,
        batch_size: usize,
    ) -> Self {
        Self {
            queue,
            handlers,
            plugin_dispatcher: None,
            pool,
            poll_interval,
            batch_size,
            visibility_timeout: Duration::from_secs(300),
            claim: JobTypeFilter::Any,
            shutdown: None,
        }
    }

    /// Attaches a shutdown signal so the worker drains and exits cleanly.
    #[must_use]
    pub fn with_shutdown(mut self, shutdown: tokio::sync::watch::Receiver<bool>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Sets the global visibility timeout used as the heartbeat cadence basis
    /// for jobs without a per-job `timeout_secs`.
    #[must_use]
    pub fn with_visibility_timeout(mut self, timeout: Duration) -> Self {
        self.visibility_timeout = timeout;
        self
    }

    /// Restricts which job types this pool claims (IO pool = `Except(cpu)`,
    /// CPU pool = `Only(cpu)`).
    #[must_use]
    pub fn with_claim_filter(mut self, claim: JobTypeFilter) -> Self {
        self.claim = claim;
        self
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

            // Graceful shutdown: stop claiming once signalled. The job currently
            // running (if any) already finished — `execute_batch` is awaited
            // below — so this only drops unstarted work, which the sweeper
            // reclaims after the visibility timeout.
            if self.shutdown.as_ref().is_some_and(|rx| *rx.borrow()) {
                tracing::info!("worker-{worker_id} shutting down (drained)");
                return;
            }

            match self
                .queue
                .dequeue_filtered(self.batch_size, &self.claim)
                .await
            {
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
        // Group jobs by their handler's coalesce key.
        use std::collections::BTreeMap;
        let mut coalesce_groups: BTreeMap<String, Vec<&super::QueuedJob>> = BTreeMap::new();

        for job in jobs {
            let handler = self.handlers.get_handler(job.job.job_type());
            let key = handler.and_then(|h| h.coalesce_key(&job.job));
            match key {
                Some(k) => coalesce_groups.entry(k).or_default().push(job),
                None => {
                    if let Err(e) = self.execute(job).await {
                        tracing::error!("worker-{worker_id} job {} error: {e}", job.id);
                    }
                }
            }
        }

        for (key, group) in coalesce_groups {
            let job_type = group[0].job.job_type().to_string();
            let Some(h) = self.handlers.get_handler(&job_type) else {
                tracing::warn!("no handler for coalesced key '{key}'");
                continue;
            };
            // A handler that defines coalesce_key but returns None from
            // coalesce must not silently drop claimed jobs — fall back to
            // executing the first job of the group.
            let merged = match h.coalesce(group.iter().map(|q| q.job.clone()).collect()) {
                Some(merged) => merged,
                None => group[0].job.clone(),
            };
            let handler_start = std::time::Instant::now();
            let heartbeat = self.spawn_heartbeat(
                group.iter().map(|q| q.id.clone()).collect(),
                heartbeat_interval(self.effective_timeout(group[0])),
            );
            let enforced = group[0]
                .timeout_secs
                .filter(|s| *s > 0)
                .map(|s| Duration::from_secs(s as u64));
            let result = run_with_timeout(enforced, self.handlers.handle(&merged)).await;
            heartbeat.abort();
            let elapsed_ms = handler_start.elapsed().as_millis() as i64;

            if let Err(e) = result {
                tracing::error!("worker-{worker_id} coalesced '{key}' error: {e}");
                for q in &group {
                    if let Err(er) = self.queue.fail(&q.id, &format!("{e}")).await {
                        tracing::error!(
                            "worker-{worker_id} failed to fail coalesced job {}: {er}",
                            q.id
                        );
                    }
                    self.writeback_cron_log(q, Err(format!("{e} (elapsed {elapsed_ms}ms)")))
                        .await;
                }
            } else {
                for q in &group {
                    if let Err(er) = self.queue.complete(&q.id).await {
                        tracing::error!(
                            "worker-{worker_id} failed to complete coalesced job {}: {er}",
                            q.id
                        );
                    }
                    self.writeback_cron_log(q, Ok(elapsed_ms)).await;
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

        // Measure handler execution time for cron log writeback.
        let handler_start = std::time::Instant::now();

        let heartbeat = self.spawn_heartbeat(
            vec![job.id.clone()],
            heartbeat_interval(self.effective_timeout(job)),
        );

        // Hard timeout only when explicitly requested on the job; jobs without
        // it are kept alive by the heartbeat instead. See §10.1 decision #3.
        let enforced = job
            .timeout_secs
            .filter(|s| *s > 0)
            .map(|s| Duration::from_secs(s as u64));

        let result = if self.handlers.has_handler(job_type) {
            run_with_timeout(enforced, self.handlers.handle_queued(job)).await
        } else if let Some(ref dispatcher) = self.plugin_dispatcher {
            tracing::info!("no built-in handler for '{job_type}', dispatching to plugins");
            run_with_timeout(enforced, dispatcher.dispatch(&job.job)).await
        } else {
            tracing::warn!("no handler for job type '{job_type}', marking dead");
            heartbeat.abort();
            self.queue.dead(&job.id, "no handler registered").await?;
            self.trace_flip(job, false, "no handler registered".to_string())
                .await;
            self.writeback_cron_log(job, Err("no handler registered".to_string()))
                .await;
            return Ok(());
        };

        heartbeat.abort();

        let elapsed_ms = handler_start.elapsed().as_millis() as i64;

        match result {
            Ok(()) => {
                self.queue.complete(&job.id).await?;
                self.trace_flip(job, true, elapsed_ms.to_string()).await;
                self.writeback_cron_log(job, Ok(elapsed_ms)).await;
            }
            Err(e) => {
                let err_msg = format!("{e}");
                let became_dead = job.attempts >= job.max_attempts;
                if became_dead {
                    self.queue.dead(&job.id, &err_msg).await?;
                } else {
                    self.queue.fail(&job.id, &err_msg).await?;
                }
                self.trace_flip(job, false, err_msg.clone()).await;
                self.writeback_cron_log(
                    job,
                    Err(format!(
                        "{err_msg} (elapsed {elapsed_ms}ms, dead={became_dead})"
                    )),
                )
                .await;
            }
        }
        Ok(())
    }

    /// Effective visibility timeout for a job: its own `timeout_secs` when set,
    /// otherwise the global `visibility_timeout`.
    fn effective_timeout(&self, job: &QueuedJob) -> Duration {
        job.timeout_secs
            .filter(|s| *s > 0)
            .map(|s| Duration::from_secs(s as u64))
            .unwrap_or(self.visibility_timeout)
    }

    /// Spawns a task that periodically bumps `updated_at` for the given running
    /// jobs. Without it, `StuckJobSweeper` reclaims a long-running job as stuck
    /// and re-dispatches it while the original execution is still in flight.
    /// The caller must `abort()` the returned handle once the handler returns.
    fn spawn_heartbeat(&self, ids: Vec<String>, interval: Duration) -> tokio::task::JoinHandle<()> {
        let pool = self.pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                for id in &ids {
                    if let Err(e) = touch_running_job(&pool, id).await {
                        tracing::warn!("heartbeat for job {id} failed: {e}");
                    }
                }
            }
        })
    }

    /// Integration-plane trace writeback: flip the receipt's pending
    /// `job:{type}` placeholder to its terminal state (§10.7). No-op for
    /// jobs without a `trace_id` payload.
    async fn trace_flip(&self, job: &super::QueuedJob, ok: bool, detail: String) {
        if let crate::worker::Job::Custom { job_type, payload } = &job.job
            && let Some(trace_id) = crate::types::snowflake_id::parse_id_value(
                payload.get("trace_id").unwrap_or(&serde_json::Value::Null),
            )
        {
            let res = crate::integration::receipt::flip_pending_step(
                &self.pool,
                crate::types::snowflake_id::SnowflakeId::new(trace_id),
                job_type,
                ok,
                &detail,
            )
            .await;
            if let Err(err) = res {
                tracing::warn!(trace_id, job_type, error = %err, "trace flip failed");
            }
        }
    }

    /// Write back the real execution outcome to `cron_execution_log`.
    ///
    /// Only called when the job has cron provenance (`cron_log_id` is Some).
    /// The `Ok(i64)` arm carries duration_ms; the `Err` arm carries the error string.
    async fn writeback_cron_log(&self, job: &super::QueuedJob, outcome: Result<i64, String>) {
        let Some(log_id) = job.cron_log_id else {
            return; // Not a cron-originated job (EventBus / ad-hoc enqueue)
        };
        let now = crate::utils::tz::now_utc();
        let log_id: SnowflakeId = log_id;
        match outcome {
            Ok(duration_ms) => {
                let res = crate::worker::complete_execution_log_with(
                    &self.pool,
                    log_id,
                    duration_ms,
                    now,
                )
                .await;
                if let Err(e) = res {
                    tracing::warn!("failed to writeback cron log {log_id}: {e}");
                }
            }
            Err(err_str) => {
                let became_dead = job.attempts >= job.max_attempts;
                let status = if became_dead {
                    CronExecStatus::Dead
                } else {
                    CronExecStatus::Failed
                };
                let res = crate::worker::fail_execution_log_with(
                    &self.pool, log_id, status, &err_str, now,
                )
                .await;
                if let Err(e) = res {
                    tracing::warn!("failed to writeback cron log {log_id}: {e}");
                }
            }
        }
    }

    fn clone_for_worker(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            handlers: self.handlers.clone(),
            plugin_dispatcher: self.plugin_dispatcher.clone(),
            pool: self.pool.clone(),
            poll_interval: self.poll_interval,
            batch_size: self.batch_size,
            visibility_timeout: self.visibility_timeout,
            claim: self.claim.clone(),
            shutdown: self.shutdown.clone(),
        }
    }
}

/// Runs `fut` under an optional hard timeout. A timeout flips the job to a
/// failure so it follows the normal retry/dead path. Note: the underlying
/// sync CPU work (e.g. `spawn_blocking` parse) is not cancellable, so it keeps
/// running in the background — the heartbeat keeps the row alive until it ends.
async fn run_with_timeout<F>(limit: Option<Duration>, fut: F) -> AppResult<()>
where
    F: std::future::Future<Output = AppResult<()>>,
{
    match limit {
        Some(d) => match tokio::time::timeout(d, fut).await {
            Ok(r) => r,
            Err(_) => Err(AppError::Internal(anyhow::anyhow!(
                "job timed out after {}s",
                d.as_secs()
            ))),
        },
        None => fut.await,
    }
}

/// Heartbeat cadence: roughly three beats per visibility window, clamped to
/// `[1s, 60s]` so short timeouts are still respected without hammering the DB.
fn heartbeat_interval(timeout: Duration) -> Duration {
    (timeout / 3).clamp(Duration::from_secs(1), Duration::from_secs(60))
}

/// Bumps `updated_at` on a still-`running` job. No-op once the job reached a
/// terminal state, so it can never resurrect a completed/failed row.
async fn touch_running_job(pool: &Pool, job_id: &str) -> AppResult<()> {
    let id: i64 = job_id
        .parse()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid id: {e}")))?;
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE jobs SET updated_at = {} WHERE {COL_ID} = {} AND status = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3)
    );
    sqlx::query::<crate::db::pool::Db>(crate::db::safe_sql(&sql))
        .bind(now)
        .bind(id)
        .bind(JobStatus::Running.as_str())
        .execute(pool)
        .await?;
    Ok(())
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

    async fn setup() -> (
        Arc<DefaultJobQueue>,
        Arc<JobHandlerRegistry>,
        crate::db::Pool,
    ) {
        let pool = crate::test_pool!();
        // Clear leftover rows from previous test runs (shared PG database).
        sqlx::query("DELETE FROM jobs")
            .execute(&pool)
            .await
            .unwrap();
        let queue = Arc::new(DefaultJobQueue::new(pool.clone()));
        let mut registry = JobHandlerRegistry::new();
        registry.register("generate_sitemap", Box::new(LogJobHandler));
        registry.register("send_welcome_email", Box::new(FailHandler));
        registry.register("rebuild_search_index", Box::new(LogJobHandler));
        (queue, Arc::new(registry), pool)
    }

    #[test]
    fn heartbeat_interval_is_clamped() {
        assert_eq!(
            heartbeat_interval(Duration::from_secs(300)),
            Duration::from_secs(60)
        );
        assert_eq!(
            heartbeat_interval(Duration::from_secs(90)),
            Duration::from_secs(30)
        );
        assert_eq!(
            heartbeat_interval(Duration::from_secs(3)),
            Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn heartbeat_does_not_resurrect_terminal_job() {
        let (queue, _registry, pool) = setup().await;
        queue
            .enqueue(NewJob::from(Job::GenerateSitemap))
            .await
            .unwrap();
        let jobs = queue.dequeue(10).await.unwrap();
        let id = jobs[0].id.clone();
        queue.complete(&id).await.unwrap();

        assert!(touch_running_job(&pool, &id).await.is_ok());

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.running, 0);
    }

    struct SlowHandler;

    #[async_trait::async_trait]
    impl crate::worker::JobHandler for SlowHandler {
        async fn handle(&self, _job: &Job) -> AppResult<()> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn execute_enforces_timeout() {
        let (queue, _registry, pool) = setup().await;
        let mut registry = JobHandlerRegistry::new();
        registry.register("slow_job", Box::new(SlowHandler));
        let runner = WorkerRunner::new(
            queue.clone(),
            Arc::new(registry),
            pool,
            Duration::from_millis(50),
            5,
        );

        queue
            .enqueue(NewJob {
                job: Job::Custom {
                    job_type: "slow_job".into(),
                    payload: serde_json::json!({}),
                },
                max_attempts: Some(3),
                run_after: None,
                cron_schedule_id: None,
                cron_log_id: None,
                priority: 0,
                timeout_secs: Some(1),
                dedup_key: None,
            })
            .await
            .unwrap();
        let jobs = queue.dequeue(10).await.unwrap();

        let start = std::time::Instant::now();
        assert!(runner.execute(&jobs[0]).await.is_ok());
        assert!(start.elapsed() < Duration::from_secs(3));

        // Timed out → retryable → back to pending (attempt 1 of 3).
        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.completed, 0);
    }

    #[tokio::test]
    async fn shutdown_stops_claiming() {
        let (queue, registry, pool) = setup().await;
        queue
            .enqueue(NewJob::from(Job::GenerateSitemap))
            .await
            .unwrap();

        let (_tx, rx) = tokio::sync::watch::channel(true);
        let runner = WorkerRunner::new(queue.clone(), registry, pool, Duration::from_millis(20), 5)
            .with_shutdown(rx);
        runner.spawn(1);
        tokio::time::sleep(Duration::from_millis(120)).await;

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.running, 0);
    }

    #[tokio::test]
    async fn execute_completes_on_handler_success() {
        let (queue, registry, pool) = setup().await;
        let runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(100),
            5,
        );

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
        let (queue, registry, pool) = setup().await;
        let runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(100),
            5,
        );

        queue
            .enqueue(NewJob {
                job: Job::SendWelcomeEmail {
                    user_id: SnowflakeId(1),
                    email: "a@b.com".into(),
                    username: "alice".into(),
                },
                max_attempts: Some(3),
                run_after: None,
                cron_schedule_id: None,
                cron_log_id: None,
                priority: 0,
                timeout_secs: None,
                dedup_key: None,
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
        let (queue, registry, pool) = setup().await;
        let runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(100),
            5,
        );

        queue
            .enqueue(NewJob {
                job: Job::SendWelcomeEmail {
                    user_id: SnowflakeId(1),
                    email: "a@b.com".into(),
                    username: "alice".into(),
                },
                max_attempts: Some(1),
                run_after: None,
                cron_schedule_id: None,
                cron_log_id: None,
                priority: 0,
                timeout_secs: None,
                dedup_key: None,
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
        let (queue, registry, pool) = setup().await;
        let _runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(100),
            5,
        );

        let jobs = queue.dequeue(10).await.unwrap();
        assert!(jobs.is_empty());

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.pending, 0);
    }

    #[tokio::test]
    async fn spawn_processes_pending_jobs() {
        let (queue, registry, pool) = setup().await;
        let runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(50),
            5,
        );

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
        let (queue, registry, pool) = setup().await;
        let runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(100),
            5,
        );

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
        let (queue, registry, pool) = setup().await;
        let runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(100),
            20,
        );

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
        let (queue, registry, pool) = setup().await;
        let runner = WorkerRunner::new(
            queue.clone(),
            registry,
            pool.clone(),
            Duration::from_millis(100),
            20,
        );

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

//! `SQLite` persisted job queue

use sqlx::Row;

use crate::constants::COL_ID;
use crate::db::Pool;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

use super::{
    JobQueue, JobRow, JobStats, JobStatus, NewJob, QueuedJob, backoff_duration, parse_job,
    serialize_job,
};
use crate::types::snowflake_id::SnowflakeId;

/// `SQLite` persisted job queue
pub struct DefaultJobQueue {
    pool: Pool,
}

impl DefaultJobQueue {
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobQueue for DefaultJobQueue {
    async fn enqueue(&self, new_job: NewJob) -> AppResult<()> {
        let id = crate::utils::id::new_id();
        let now = crate::utils::tz::now_utc();
        let job_type = new_job.job.job_type();
        let payload = serialize_job(&new_job.job);
        let max_attempts = new_job.max_attempts.unwrap_or(3) as i32;

        // Dedup: skip enqueue if a pending job with the same dedup_key exists.
        if let Some(ref key) = new_job.dedup_key {
            let exists: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE dedup_key = {} AND status = {})",
                Driver::ph(1),
                Driver::ph(2)
            ))
            .bind(key)
            .bind(JobStatus::Pending)
            .fetch_one(&self.pool)
            .await?;
            if exists {
                tracing::debug!("dedup: job {job_type} with key '{key}' already pending, skipping");
                return Ok(());
            }
        }

        raisfast_derive::crud_insert!(&self.pool, "jobs", [
            "id" => id,
            "job_type" => job_type,
            "payload" => &payload,
            "status" => JobStatus::Pending.as_str(),
            "max_attempts" => max_attempts,
            "run_after" => new_job.run_after,
            "cron_schedule_id" => new_job.cron_schedule_id,
            "cron_log_id" => new_job.cron_log_id,
            "priority" => new_job.priority,
            "timeout_secs" => new_job.timeout_secs,
            "dedup_key" => new_job.dedup_key,
            "created_at" => now,
            "updated_at" => now
        ])?;

        tracing::debug!("enqueued job {id} type={job_type}");
        Ok(())
    }

    async fn dequeue(&self, limit: usize) -> AppResult<Vec<QueuedJob>> {
        let now = crate::utils::tz::now_utc();
        let limit_i64 = limit as i64;

        #[cfg(feature = "db-postgres")]
        {
            // PostgreSQL: atomically claim pending jobs with FOR UPDATE SKIP LOCKED.
            // The UPDATE..IN(SELECT..LIMIT) approach has a race: two concurrent
            // workers can both SELECT the same pending job and both UPDATE it
            // (Postgres UPDATE does not skip rows locked by the subquery), so the
            // job gets executed twice. SKIP LOCKED makes the claim atomic.
            let mut tx = self.pool.begin().await?;

            let select_sql = format!(
                "SELECT {COL_ID} FROM jobs \
                 WHERE status = {} AND (run_after IS NULL OR run_after <= {}) \
                 ORDER BY priority DESC, created_at ASC LIMIT {} \
                 FOR UPDATE SKIP LOCKED",
                Driver::ph(1),
                Driver::ph(2),
                Driver::ph(3)
            );
            let ids: Vec<i64> = sqlx::query_scalar::<_, i64>(&select_sql)
                .bind(JobStatus::Pending)
                .bind(now)
                .bind(limit_i64)
                .fetch_all(&mut *tx)
                .await?;

            let mut jobs = Vec::with_capacity(ids.len());
            for id in ids {
                let update_sql = format!(
                    "UPDATE jobs SET status = {}, attempts = attempts + 1, updated_at = {} \
                     WHERE {COL_ID} = {} \
                     RETURNING {COL_ID}, job_type, payload, attempts, max_attempts, created_at, cron_schedule_id, cron_log_id",
                    Driver::ph(1),
                    Driver::ph(2),
                    Driver::ph(3)
                );
                let row: Option<crate::db::pool::DbRow> =
                    sqlx::query::<crate::db::pool::Db>(&update_sql)
                        .bind(JobStatus::Running)
                        .bind(now)
                        .bind(id)
                        .fetch_optional(&mut *tx)
                        .await?;

                let Some(r) = row else {
                    continue;
                };
                let id: i64 = r.get::<i64, _>("id");
                let job_type: String = r.get::<String, _>("job_type");
                let payload: String = r.get::<String, _>("payload");
                let attempts: i32 = r.get::<i32, _>("attempts");
                let max_attempts: i32 = r.get::<i32, _>("max_attempts");
                let created_at: Timestamp = r.get::<Timestamp, _>("created_at");
                let cron_schedule_id: Option<i64> = r.get::<Option<i64>, _>("cron_schedule_id");
                let cron_log_id: Option<i64> = r.get::<Option<i64>, _>("cron_log_id");
                match parse_job(&job_type, &payload) {
                    Ok(job) => jobs.push(QueuedJob {
                        id: id.to_string(),
                        job,
                        attempts: attempts as u32,
                        max_attempts: max_attempts as u32,
                        created_at,
                        cron_schedule_id: cron_schedule_id.map(SnowflakeId),
                        cron_log_id: cron_log_id.map(SnowflakeId),
                    }),
                    Err(e) => {
                        tracing::error!("failed to parse job {id}: {e}");
                        let _ = self
                            .dead(&id.to_string(), &format!("parse error: {e}"))
                            .await;
                    }
                }
            }

            tx.commit().await?;
            Ok(jobs)
        }

        #[cfg(all(feature = "db-sqlite", not(feature = "db-mysql")))]
        {
            // SQLite: single-writer model with global write lock, UPDATE is atomic.
            let returning = crate::db::Driver::returning_col(&format!(
                "{COL_ID}, job_type, payload, attempts, max_attempts, created_at, cron_schedule_id, cron_log_id"
            ));
            let sql = format!(
                "UPDATE jobs SET status = {}, attempts = attempts + 1, updated_at = {}
                 WHERE {COL_ID} IN (
                   SELECT {COL_ID} FROM jobs
                   WHERE status = {} AND (run_after IS NULL OR run_after <= {})
                   ORDER BY priority DESC, created_at ASC LIMIT {}
                 )
                 {returning}",
                Driver::ph(1),
                Driver::ph(2),
                Driver::ph(3),
                Driver::ph(4),
                Driver::ph(5)
            );

            let rows: Vec<crate::db::pool::DbRow> = sqlx::query::<crate::db::pool::Db>(&sql)
                .bind(JobStatus::Running)
                .bind(now)
                .bind(JobStatus::Pending)
                .bind(now)
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?;

            let mut jobs = Vec::with_capacity(rows.len());
            for row in rows {
                let id: i64 = row.get::<Option<i64>, _>("id").unwrap_or_default();
                let job_type: String = row.get::<String, _>("job_type");
                let payload: String = row.get::<String, _>("payload");
                let attempts: i32 = row.get::<i32, _>("attempts");
                let max_attempts: i32 = row.get::<i32, _>("max_attempts");
                let created_at: Timestamp = row.get::<Timestamp, _>("created_at");
                let cron_schedule_id: Option<i64> = row.get::<Option<i64>, _>("cron_schedule_id");
                let cron_log_id: Option<i64> = row.get::<Option<i64>, _>("cron_log_id");
                match parse_job(&job_type, &payload) {
                    Ok(job) => jobs.push(QueuedJob {
                        id: id.to_string(),
                        job,
                        attempts: attempts as u32,
                        max_attempts: max_attempts as u32,
                        created_at,
                        cron_schedule_id: cron_schedule_id.map(SnowflakeId),
                        cron_log_id: cron_log_id.map(SnowflakeId),
                    }),
                    Err(e) => {
                        tracing::error!("failed to parse job {id}: {e}");
                        let _ = self
                            .dead(&id.to_string(), &format!("parse error: {e}"))
                            .await;
                    }
                }
            }

            Ok(jobs)
        }

        #[cfg(feature = "db-mysql")]
        {
            // MySQL lacks RETURNING, but supports `FOR UPDATE SKIP LOCKED` (8.0+).
            // Single transaction: SELECT ... FOR UPDATE SKIP LOCKED → UPDATE → fetch.
            // This is atomic, race-free, and avoids the N+1 query problem.
            let mut tx = self.pool.begin().await?;

            let select_sql = format!(
                "SELECT {COL_ID}, job_type, payload, attempts, max_attempts, created_at, cron_schedule_id, cron_log_id \
                 FROM jobs \
                 WHERE status = {} AND (run_after IS NULL OR run_after <= {}) \
                 ORDER BY priority DESC, created_at ASC \
                 LIMIT {} \
                 FOR UPDATE SKIP LOCKED",
                Driver::ph(1),
                Driver::ph(2),
                Driver::ph(3)
            );
            let rows: Vec<crate::db::pool::DbRow> = sqlx::query::<crate::db::pool::Db>(&select_sql)
                .bind(JobStatus::Pending)
                .bind(now)
                .bind(limit_i64)
                .fetch_all(&mut *tx)
                .await?;

            if rows.is_empty() {
                tx.commit().await?;
                return Ok(Vec::new());
            }

            // Collect IDs for a single bulk UPDATE
            let ids: Vec<i64> = rows.iter().map(|r| r.get::<i64, _>(COL_ID)).collect();

            // Bulk UPDATE all selected rows at once (they're already locked by FOR UPDATE)
            let placeholders: Vec<String> = (0..ids.len()).map(|i| Driver::ph(4 + i)).collect();
            let update_sql = format!(
                "UPDATE jobs SET status = {}, attempts = attempts + 1, updated_at = {} \
                 WHERE {COL_ID} IN ({}) AND status = {}",
                Driver::ph(1),
                Driver::ph(2),
                placeholders.join(", "),
                Driver::ph(3)
            );

            let mut q = sqlx::query::<crate::db::pool::Db>(&update_sql)
                .bind(JobStatus::Running)
                .bind(now)
                .bind(JobStatus::Pending);
            for id in &ids {
                q = q.bind(id);
            }
            q.execute(&mut *tx).await?;

            tx.commit().await?;

            let mut jobs = Vec::with_capacity(rows.len());
            for row in rows {
                let id: i64 = row.get::<i64, _>(COL_ID);
                let job_type: String = row.get::<String, _>("job_type");
                let payload: String = row.get::<String, _>("payload");
                let attempts: i32 = row.get::<i32, _>("attempts");
                let max_attempts: i32 = row.get::<i32, _>("max_attempts");
                let created_at: Timestamp = row.get::<Timestamp, _>("created_at");
                let cron_schedule_id: Option<i64> = row.get::<Option<i64>, _>("cron_schedule_id");
                let cron_log_id: Option<i64> = row.get::<Option<i64>, _>("cron_log_id");

                match parse_job(&job_type, &payload) {
                    Ok(job) => jobs.push(QueuedJob {
                        id: id.to_string(),
                        job,
                        attempts: (attempts + 1) as u32, // +1 because bulk UPDATE incremented it
                        max_attempts: max_attempts as u32,
                        created_at,
                        cron_schedule_id: cron_schedule_id.map(SnowflakeId),
                        cron_log_id: cron_log_id.map(SnowflakeId),
                    }),
                    Err(e) => {
                        tracing::error!("failed to parse job {id}: {e}");
                        let _ = self
                            .dead(&id.to_string(), &format!("parse error: {e}"))
                            .await;
                    }
                }
            }

            Ok(jobs)
        }
    }

    async fn complete(&self, id: &str) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();
        let id: i64 = id
            .parse()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid id: {e}")))?;
        raisfast_derive::crud_update!(&self.pool, "jobs",
            bind: ["status" => JobStatus::Completed, "updated_at" => now],
            where: ("id", id)
        )?;
        tracing::debug!("job {id} completed");
        Ok(())
    }

    async fn fail(&self, id: &str, error: &str) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();
        let id: i64 = id
            .parse()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid id: {e}")))?;

        in_transaction!(&self.pool, tx, {
            let sql = format!(
                "SELECT attempts, max_attempts FROM jobs WHERE {COL_ID} = {}",
                Driver::ph(1)
            );
            let row: Option<crate::db::pool::DbRow> = sqlx::query::<crate::db::pool::Db>(&sql)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;

            let Some(r) = row else {
                return Err(AppError::not_found("job"));
            };

            let attempts: i32 = r.get::<i32, _>("attempts");
            let max_attempts: i32 = r.get::<i32, _>("max_attempts");

            if attempts >= max_attempts {
                raisfast_derive::crud_update!(&mut *tx, "jobs",
                    bind: ["status" => JobStatus::Dead, "error" => error, "updated_at" => now],
                    where: ("id", id)
                )?;
                tracing::error!("job {id} dead: {error}");
                return Ok::<_, AppError>(());
            }

            let delay = backoff_duration(attempts as u32);
            let run_after =
                crate::utils::tz::now_utc() + chrono::Duration::from_std(delay).unwrap_or_default();

            raisfast_derive::crud_update!(&mut *tx, "jobs",
                bind: ["status" => JobStatus::Pending, "error" => error, "run_after" => run_after, "updated_at" => now],
                where: ("id", id)
            )?;

            tracing::warn!(
                "job {id} failed (attempt {attempts}/{max_attempts}), retry after {run_after}"
            );
            Ok(())
        })
    }

    async fn dead(&self, id: &str, error: &str) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();
        let id: i64 = id
            .parse()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid id: {e}")))?;
        raisfast_derive::crud_update!(&self.pool, "jobs",
            bind: ["status" => JobStatus::Dead, "error" => error, "updated_at" => now],
            where: ("id", id)
        )?;
        tracing::error!("job {id} dead: {error}");
        Ok(())
    }

    async fn stats(&self) -> AppResult<JobStats> {
        let row: crate::db::pool::DbRow = sqlx::query::<crate::db::pool::Db>(&format!(
            "SELECT
                COALESCE(SUM(CASE WHEN status={} THEN 1 ELSE 0 END), 0) as pending,
                COALESCE(SUM(CASE WHEN status={} THEN 1 ELSE 0 END), 0) as running,
                COALESCE(SUM(CASE WHEN status={} THEN 1 ELSE 0 END), 0) as completed,
                COALESCE(SUM(CASE WHEN status={} THEN 1 ELSE 0 END), 0) as failed,
                COALESCE(SUM(CASE WHEN status={} THEN 1 ELSE 0 END), 0) as dead
             FROM jobs",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(4),
            Driver::ph(5)
        ))
        .bind(JobStatus::Pending)
        .bind(JobStatus::Running)
        .bind(JobStatus::Completed)
        .bind(JobStatus::Failed)
        .bind(JobStatus::Dead)
        .fetch_one(&self.pool)
        .await?;

        Ok(JobStats {
            pending: row.get("pending"),
            running: row.get("running"),
            completed: row.get("completed"),
            failed: row.get("failed"),
            dead: row.get("dead"),
        })
    }

    async fn list(
        &self,
        status: Option<JobStatus>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<JobRow>, i64)> {
        let offset = (page - 1) * page_size;

        let (items, total): (Vec<JobRow>, i64) = if let Some(s) = status {
            let rows: Vec<crate::db::pool::DbRow> = sqlx::query::<crate::db::pool::Db>(&format!(
                "SELECT {COL_ID}, job_type, payload, status, attempts, max_attempts, run_after, error, created_at, updated_at
                 FROM jobs WHERE status = {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
                Driver::ph(1), Driver::ph(2), Driver::ph(3)
            ))
            .bind(s)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

            let total: i64 = sqlx::query_scalar::<crate::db::pool::Db, i64>(&format!(
                "SELECT COUNT(*) FROM jobs WHERE status = {}",
                Driver::ph(1)
            ))
            .bind(s)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

            let items = rows
                .into_iter()
                .map(|r: crate::db::pool::DbRow| JobRow {
                    id: r
                        .get::<Option<i64>, _>(COL_ID)
                        .map(|i: i64| i.to_string())
                        .unwrap_or_default(),
                    job_type: r.get::<String, _>("job_type"),
                    payload: r.get::<String, _>("payload"),
                    status: r.get::<super::JobStatus, _>("status"),
                    attempts: r.get::<i32, _>("attempts") as u32,
                    max_attempts: r.get::<i32, _>("max_attempts") as u32,
                    run_after: r.get::<Option<crate::utils::tz::Timestamp>, _>("run_after"),
                    error: r.get::<Option<String>, _>("error"),
                    created_at: r.get::<crate::utils::tz::Timestamp, _>("created_at"),
                    updated_at: r.get::<crate::utils::tz::Timestamp, _>("updated_at"),
                })
                .collect();

            (items, total)
        } else {
            let rows: Vec<crate::db::pool::DbRow> = sqlx::query::<crate::db::pool::Db>(&format!(
                "SELECT {COL_ID}, job_type, payload, status, attempts, max_attempts, run_after, error, created_at, updated_at
                 FROM jobs ORDER BY created_at DESC LIMIT {} OFFSET {}",
                Driver::ph(1), Driver::ph(2)
            ))
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

            let total: i64 =
                sqlx::query_scalar::<crate::db::pool::Db, i64>("SELECT COUNT(*) FROM jobs")
                    .fetch_one(&self.pool)
                    .await
                    .unwrap_or(0);

            let items = rows
                .into_iter()
                .map(|r: crate::db::pool::DbRow| JobRow {
                    id: r
                        .get::<Option<i64>, _>(COL_ID)
                        .map(|i: i64| i.to_string())
                        .unwrap_or_default(),
                    job_type: r.get::<String, _>("job_type"),
                    payload: r.get::<String, _>("payload"),
                    status: r.get::<super::JobStatus, _>("status"),
                    attempts: r.get::<i32, _>("attempts") as u32,
                    max_attempts: r.get::<i32, _>("max_attempts") as u32,
                    run_after: r.get::<Option<crate::utils::tz::Timestamp>, _>("run_after"),
                    error: r.get::<Option<String>, _>("error"),
                    created_at: r.get::<crate::utils::tz::Timestamp, _>("created_at"),
                    updated_at: r.get::<crate::utils::tz::Timestamp, _>("updated_at"),
                })
                .collect();

            (items, total)
        };

        Ok((items, total))
    }

    async fn retry(&self, id: &str) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();
        let id: i64 = id
            .parse()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid id: {e}")))?;
        let result: crate::db::pool::DbQueryResult = raisfast_derive::crud_update!(&self.pool, "jobs",
            bind: [
            "status" => JobStatus::Pending.as_str(),
                "attempts" => 0i32,
                "error" => None::<String>,
                "run_after" => None::<crate::utils::tz::Timestamp>,
                "updated_at" => now
            ],
            where: AND(("id", id), ("status", JobStatus::Dead))
        )?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found("job"));
        }

        tracing::info!("job {id} retried (reset to pending)");
        Ok(())
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        let id: i64 = id
            .parse()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid id: {e}")))?;
        let result: crate::db::DbQueryResult =
            raisfast_derive::crud_delete!(&self.pool, "jobs", where: ("id", id))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found("job"));
        }

        Ok(())
    }

    async fn cleanup(&self) -> AppResult<u64> {
        let sql = format!(
            "DELETE FROM jobs WHERE status IN ({}, {}) AND updated_at < {}",
            Driver::ph(1),
            Driver::ph(2),
            crate::db::Driver::ago_expr(7)
        );
        let result: crate::db::pool::DbQueryResult = sqlx::query::<crate::db::pool::Db>(&sql)
            .bind(JobStatus::Completed)
            .bind(JobStatus::Dead)
            .execute(&self.pool)
            .await?;

        let count = result.rows_affected();
        if count > 0 {
            tracing::info!("cleaned up {count} old jobs");
        }
        Ok(count)
    }

    async fn requeue_stuck(&self, timeout: std::time::Duration) -> AppResult<u64> {
        let now = crate::utils::tz::now_utc();
        let timeout_secs = timeout.as_secs() as i64;
        let cutoff_expr =
            crate::db::Driver::ago_seconds_expr(&format!("COALESCE(timeout_secs, {timeout_secs})"));

        // Jobs that exhausted retries → dead
        let dead_sql = format!(
            "UPDATE jobs SET status = {}, error = {}, updated_at = {} \
             WHERE status = {} AND updated_at < {} AND attempts >= max_attempts",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(4),
            cutoff_expr
        );
        let dead_result: crate::db::pool::DbQueryResult =
            sqlx::query::<crate::db::pool::Db>(&dead_sql)
                .bind(JobStatus::Dead)
                .bind("visibility timeout exceeded")
                .bind(now)
                .bind(JobStatus::Running)
                .execute(&self.pool)
                .await?;
        let dead_count = dead_result.rows_affected();

        // Jobs still retryable → back to pending
        let requeue_sql = format!(
            "UPDATE jobs SET status = {}, error = {}, updated_at = {} \
             WHERE status = {} AND updated_at < {} AND attempts < max_attempts",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(4),
            cutoff_expr
        );
        let requeue_result: crate::db::pool::DbQueryResult =
            sqlx::query::<crate::db::pool::Db>(&requeue_sql)
                .bind(JobStatus::Pending)
                .bind("visibility timeout exceeded")
                .bind(now)
                .bind(JobStatus::Running)
                .execute(&self.pool)
                .await?;
        let requeue_count = requeue_result.rows_affected();

        let total = dead_count + requeue_count;
        if total > 0 {
            tracing::warn!(
                "requeued {requeue_count} stuck jobs, marked {dead_count} dead \
                 (visibility timeout {}s)",
                timeout.as_secs()
            );
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;
    use crate::worker::{Job, NewJob};

    async fn setup() -> DefaultJobQueue {
        let pool = crate::test_pool!();
        // Clear leftover rows from previous test runs (shared PG database).
        sqlx::query("DELETE FROM jobs")
            .execute(&pool)
            .await
            .unwrap();
        DefaultJobQueue::new(pool)
    }

    fn sample_job() -> NewJob {
        NewJob {
            job: Job::GenerateSitemap,
            max_attempts: Some(3),
            run_after: None,
            cron_schedule_id: None,
            cron_log_id: None,
            priority: 0,
            timeout_secs: None,
            dedup_key: None,
        }
    }

    #[tokio::test]
    async fn enqueue_and_dequeue() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();
        let jobs = q.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job.job_type(), "generate_sitemap");
        assert_eq!(jobs[0].attempts, 1);
        assert_eq!(jobs[0].max_attempts, 3);
    }

    #[tokio::test]
    async fn dequeue_changes_status_to_running() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();

        let _ = q.dequeue(10).await.unwrap();
        let second = q.dequeue(10).await.unwrap();
        assert!(second.is_empty());
    }

    #[tokio::test]
    async fn dequeue_respects_limit() {
        let q = setup().await;
        for _ in 0..5 {
            q.enqueue(sample_job()).await.unwrap();
        }
        let jobs = q.dequeue(2).await.unwrap();
        assert_eq!(jobs.len(), 2);
    }

    #[tokio::test]
    async fn complete_removes_from_pending() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();
        let jobs = q.dequeue(10).await.unwrap();

        q.complete(&jobs[0].id).await.unwrap();

        let stats = q.stats().await.unwrap();
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.running, 0);
    }

    #[tokio::test]
    async fn fail_resets_to_pending_with_backoff() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();
        let jobs = q.dequeue(10).await.unwrap();

        q.fail(&jobs[0].id, "something went wrong").await.unwrap();

        let stats = q.stats().await.unwrap();
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.running, 0);

        let (rows, _) = q.list(None, 1, 10).await.unwrap();
        assert!(rows[0].run_after.is_some());
        assert_eq!(rows[0].error.as_deref(), Some("something went wrong"));
    }

    #[tokio::test]
    async fn fail_marks_dead_after_max_attempts() {
        let q = setup().await;
        q.enqueue(NewJob {
            job: Job::GenerateSitemap,
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

        let jobs = q.dequeue(10).await.unwrap();
        assert_eq!(jobs[0].attempts, 1);
        assert_eq!(jobs[0].max_attempts, 1);

        q.fail(&jobs[0].id, "permanent failure").await.unwrap();

        let stats = q.stats().await.unwrap();
        assert_eq!(stats.dead, 1);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.running, 0);
    }

    #[tokio::test]
    async fn dead_marks_job_as_dead() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();
        let jobs = q.dequeue(10).await.unwrap();

        q.dead(&jobs[0].id, "fatal").await.unwrap();

        let stats = q.stats().await.unwrap();
        assert_eq!(stats.dead, 1);
    }

    #[tokio::test]
    async fn dead_returns_not_found_for_missing_job() {
        let q = setup().await;
        let result = q.dead("99999999", "err").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn stats_counts_by_status() {
        let q = setup().await;

        q.enqueue(sample_job()).await.unwrap();
        q.enqueue(sample_job()).await.unwrap();
        q.enqueue(sample_job()).await.unwrap();

        let jobs = q.dequeue(1).await.unwrap();
        q.complete(&jobs[0].id).await.unwrap();

        let stats = q.stats().await.unwrap();
        assert_eq!(stats.pending, 2);
        assert_eq!(stats.running, 0);
        assert_eq!(stats.completed, 1);
    }

    #[tokio::test]
    async fn list_all_jobs() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();
        q.enqueue(NewJob {
            job: Job::GenerateSitemap,
            max_attempts: Some(5),
            run_after: None,
            cron_schedule_id: None,
            cron_log_id: None,
            priority: 0,
            timeout_secs: None,
            dedup_key: None,
        })
        .await
        .unwrap();

        let (rows, total) = q.list(None, 1, 10).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn list_filter_by_status() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();
        q.enqueue(sample_job()).await.unwrap();

        let jobs = q.dequeue(1).await.unwrap();
        q.complete(&jobs[0].id).await.unwrap();

        let (pending, _) = q.list(Some(JobStatus::Pending), 1, 10).await.unwrap();
        assert_eq!(pending.len(), 1);

        let (completed, _) = q.list(Some(JobStatus::Completed), 1, 10).await.unwrap();
        assert_eq!(completed.len(), 1);
    }

    #[tokio::test]
    async fn list_pagination() {
        let q = setup().await;
        for _ in 0..5 {
            q.enqueue(sample_job()).await.unwrap();
        }

        let (page1, total) = q.list(None, 1, 2).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 2);

        let (page2, _) = q.list(None, 2, 2).await.unwrap();
        assert_eq!(page2.len(), 2);

        let (page3, _) = q.list(None, 3, 2).await.unwrap();
        assert_eq!(page3.len(), 1);
    }

    #[tokio::test]
    async fn retry_resets_dead_job() {
        let q = setup().await;
        q.enqueue(NewJob {
            job: Job::GenerateSitemap,
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

        let jobs = q.dequeue(10).await.unwrap();
        q.fail(&jobs[0].id, "err").await.unwrap();

        let stats = q.stats().await.unwrap();
        assert_eq!(stats.dead, 1);

        q.retry(&jobs[0].id).await.unwrap();

        let stats = q.stats().await.unwrap();
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.dead, 0);

        let (rows, _) = q.list(None, 1, 10).await.unwrap();
        assert_eq!(rows[0].attempts, 0);
        assert!(rows[0].error.is_none());
        assert!(rows[0].run_after.is_none());
    }

    #[tokio::test]
    async fn retry_non_dead_returns_not_found() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();
        let jobs = q.dequeue(10).await.unwrap();

        let result = q.retry(&jobs[0].id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retry_nonexistent_returns_not_found() {
        let q = setup().await;
        let result = q.retry("99999999").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn remove_deletes_job() {
        let q = setup().await;
        q.enqueue(sample_job()).await.unwrap();

        let (rows, _) = q.list(None, 1, 10).await.unwrap();
        assert_eq!(rows.len(), 1);

        q.remove(&rows[0].id).await.unwrap();

        let (rows, _) = q.list(None, 1, 10).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_not_found() {
        let q = setup().await;
        let result = q.remove("99999999").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn dequeue_skips_future_run_after() {
        let q = setup().await;
        let future = crate::utils::tz::now_utc() + chrono::Duration::hours(1);
        q.enqueue(NewJob {
            job: Job::GenerateSitemap,
            max_attempts: Some(3),
            run_after: Some(future),
            cron_schedule_id: None,
            cron_log_id: None,
            priority: 0,
            timeout_secs: None,
            dedup_key: None,
        })
        .await
        .unwrap();

        let jobs = q.dequeue(10).await.unwrap();
        assert!(jobs.is_empty());
    }

    #[tokio::test]
    async fn enqueue_multiple_job_types() {
        let q = setup().await;
        q.enqueue(NewJob::from(Job::SendWelcomeEmail {
            user_id: SnowflakeId(1),
            email: "a@b.com".into(),
            username: "alice".into(),
        }))
        .await
        .unwrap();
        q.enqueue(NewJob::from(Job::RebuildSearchIndex { post_ids: vec![1] }))
            .await
            .unwrap();
        q.enqueue(NewJob::from(Job::GenerateThumbnail {
            media_id: SnowflakeId(1),
            size: 300,
        }))
        .await
        .unwrap();

        let jobs = q.dequeue(10).await.unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].job.job_type(), "send_welcome_email");
        assert_eq!(jobs[1].job.job_type(), "rebuild_search_index");
        assert_eq!(jobs[2].job.job_type(), "generate_thumbnail");
    }

    #[tokio::test]
    async fn fail_on_nonexistent_returns_not_found() {
        let q = setup().await;
        let result = q.fail("99999999", "err").await;
        assert!(result.is_err());
    }
}

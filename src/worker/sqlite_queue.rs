//! `SQLite` 持久化任务队列

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::Pool;
use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};

use super::{
    JobQueue, JobRow, JobStats, NewJob, QueuedJob, backoff_duration, parse_job, serialize_job,
};

/// `SQLite` 持久化任务队列
pub struct SqliteJobQueue {
    pool: Pool,
}

impl SqliteJobQueue {
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobQueue for SqliteJobQueue {
    async fn enqueue(&self, new_job: NewJob) -> AppResult<()> {
        let id = Uuid::now_v7().to_string();
        let job_type = new_job.job.job_type();
        let payload = serialize_job(&new_job.job);
        let max_attempts = new_job.max_attempts.unwrap_or(3);
        let now = Utc::now().to_rfc3339();

        sqlx::query(&format!(
            "INSERT INTO jobs (document_id, job_type, payload, status, max_attempts, run_after, created_at, updated_at)
             VALUES ({}, {}, {}, 'pending', {}, {}, {}, {})",
            ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7)
        ))
        .bind(&id)
        .bind(job_type)
        .bind(&payload)
        .bind(max_attempts)
        .bind(&new_job.run_after)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        tracing::debug!("enqueued job {id} type={job_type}");
        Ok(())
    }

    async fn dequeue(&self, limit: usize) -> AppResult<Vec<QueuedJob>> {
        let now = Utc::now().to_rfc3339();
        let limit_i64 = limit as i64;

        let returning = crate::db::dialect::returning_col(
            "document_id as id, job_type, payload, attempts, max_attempts, created_at",
        );
        let sql = format!(
            "UPDATE jobs SET status = 'running', attempts = attempts + 1, updated_at = {}
             WHERE id IN (
               SELECT id FROM jobs
               WHERE status = 'pending' AND (run_after IS NULL OR run_after <= {})
               ORDER BY created_at ASC LIMIT {}
             )
             {returning}",
            ph(1),
            ph(2),
            ph(3)
        );

        let rows = sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await?;

        let mut jobs = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get::<Option<String>, _>("id").unwrap_or_default();
            let job_type: String = row.get("job_type");
            let payload: String = row.get("payload");
            let attempts: i32 = row.get("attempts");
            let max_attempts: i32 = row.get("max_attempts");
            let created_at: String = row.get("created_at");
            match parse_job(&job_type, &payload) {
                Ok(job) => jobs.push(QueuedJob {
                    id,
                    job,
                    attempts: attempts as u32,
                    max_attempts: max_attempts as u32,
                    created_at,
                }),
                Err(e) => {
                    tracing::error!("failed to parse job {id}: {e}");
                    let _ = self.dead(&id, &format!("parse error: {e}")).await;
                }
            }
        }

        Ok(jobs)
    }

    async fn complete(&self, id: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(&format!(
            "UPDATE jobs SET status = 'completed', updated_at = {} WHERE document_id = {}",
            ph(1),
            ph(2)
        ))
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        tracing::debug!("job {id} completed");
        Ok(())
    }

    async fn fail(&self, id: &str, error: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(&format!(
            "SELECT attempts, max_attempts FROM jobs WHERE document_id = {}",
            ph(1)
        ))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(r) = row else {
            return Err(AppError::not_found("job"));
        };

        let attempts: i32 = r.get("attempts");
        let max_attempts: i32 = r.get("max_attempts");

        if attempts >= max_attempts {
            sqlx::query(&format!(
            "UPDATE jobs SET status = 'dead', error = {}, updated_at = {} WHERE document_id = {}",
                ph(1),
                ph(2),
                ph(3)
            ))
            .bind(error)
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            tracing::error!("job {id} dead: {error}");
            return Ok(());
        }

        let delay = backoff_duration(attempts as u32);
        let run_after =
            (Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default()).to_rfc3339();

        sqlx::query(&format!(
            "UPDATE jobs SET status = 'pending', error = {}, run_after = {}, updated_at = {} WHERE document_id = {}",
            ph(1), ph(2), ph(3), ph(4)
        ))
        .bind(error)
        .bind(&run_after)
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        tracing::warn!(
            "job {id} failed (attempt {attempts}/{max_attempts}), retry after {run_after}"
        );
        Ok(())
    }

    async fn dead(&self, id: &str, error: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(&format!(
            "UPDATE jobs SET status = 'dead', error = {}, updated_at = {} WHERE document_id = {}",
            ph(1),
            ph(2),
            ph(3)
        ))
        .bind(error)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        tracing::error!("job {id} dead: {error}");
        Ok(())
    }

    async fn stats(&self) -> AppResult<JobStats> {
        let row = sqlx::query(
            "SELECT
                COALESCE(SUM(CASE WHEN status='pending' THEN 1 ELSE 0 END), 0) as pending,
                COALESCE(SUM(CASE WHEN status='running' THEN 1 ELSE 0 END), 0) as running,
                COALESCE(SUM(CASE WHEN status='completed' THEN 1 ELSE 0 END), 0) as completed,
                COALESCE(SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END), 0) as failed,
                COALESCE(SUM(CASE WHEN status='dead' THEN 1 ELSE 0 END), 0) as dead
             FROM jobs",
        )
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
        status: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<JobRow>, i64)> {
        let offset = (page - 1) * page_size;

        let (items, total): (Vec<JobRow>, i64) = if let Some(s) = status {
            let rows = sqlx::query(&format!(
                "SELECT document_id as id, job_type, payload, status, attempts, max_attempts, run_after, error, created_at, updated_at
                 FROM jobs WHERE status = {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
                ph(1), ph(2), ph(3)
            ))
            .bind(s)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

            let total: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM jobs WHERE status = {}",
                ph(1)
            ))
            .bind(s)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

            let items = rows
                .into_iter()
                .map(|r| JobRow {
                    id: r.get::<Option<String>, _>("id").unwrap_or_default(),
                    job_type: r.get("job_type"),
                    payload: r.get("payload"),
                    status: r.get("status"),
                    attempts: r.get::<i32, _>("attempts") as u32,
                    max_attempts: r.get::<i32, _>("max_attempts") as u32,
                    run_after: r.get("run_after"),
                    error: r.get("error"),
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                })
                .collect();

            (items, total)
        } else {
            let rows = sqlx::query(&format!(
                "SELECT document_id as id, job_type, payload, status, attempts, max_attempts, run_after, error, created_at, updated_at
                 FROM jobs ORDER BY created_at DESC LIMIT {} OFFSET {}",
                ph(1), ph(2)
            ))
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
                .fetch_one(&self.pool)
                .await
                .unwrap_or(0);

            let items = rows
                .into_iter()
                .map(|r| JobRow {
                    id: r.get::<Option<String>, _>("id").unwrap_or_default(),
                    job_type: r.get("job_type"),
                    payload: r.get("payload"),
                    status: r.get("status"),
                    attempts: r.get::<i32, _>("attempts") as u32,
                    max_attempts: r.get::<i32, _>("max_attempts") as u32,
                    run_after: r.get("run_after"),
                    error: r.get("error"),
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                })
                .collect();

            (items, total)
        };

        Ok((items, total))
    }

    async fn retry(&self, id: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(&format!(
            "UPDATE jobs SET status = 'pending', attempts = 0, error = NULL, run_after = NULL, updated_at = {}
             WHERE document_id = {} AND status = 'dead'",
            ph(1), ph(2)
        ))
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found("job"));
        }

        tracing::info!("job {id} retried (reset to pending)");
        Ok(())
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        let result = sqlx::query(&format!("DELETE FROM jobs WHERE document_id = {}", ph(1)))
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found("job"));
        }

        Ok(())
    }

    async fn cleanup(&self) -> AppResult<u64> {
        let sql = format!(
            "DELETE FROM jobs WHERE status IN ('completed', 'dead') AND updated_at < {}",
            crate::db::dialect::ago_expr(7)
        );
        let result = sqlx::query(&sql).execute(&self.pool).await?;

        let count = result.rows_affected();
        if count > 0 {
            tracing::info!("cleaned up {count} old jobs");
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{Job, NewJob};

    async fn setup() -> SqliteJobQueue {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        SqliteJobQueue::new(pool)
    }

    fn sample_job() -> NewJob {
        NewJob {
            job: Job::GenerateSitemap,
            max_attempts: Some(3),
            run_after: None,
        }
    }

    #[tokio::test]
    async fn enqueue_and_dequeue() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let q = SqliteJobQueue::new(pool);
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
        let result = q.dead("nonexistent", "err").await;
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

        let (pending, _) = q.list(Some("pending"), 1, 10).await.unwrap();
        assert_eq!(pending.len(), 1);

        let (completed, _) = q.list(Some("completed"), 1, 10).await.unwrap();
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
        let result = q.retry("nonexistent").await;
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
        let result = q.remove("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn dequeue_skips_future_run_after() {
        let q = setup().await;
        let future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        q.enqueue(NewJob {
            job: Job::GenerateSitemap,
            max_attempts: Some(3),
            run_after: Some(future),
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
            user_id: "u1".into(),
            email: "a@b.com".into(),
            username: "alice".into(),
        }))
        .await
        .unwrap();
        q.enqueue(NewJob::from(Job::RebuildSearchIndex {
            post_ids: vec!["p1".into()],
        }))
        .await
        .unwrap();
        q.enqueue(NewJob::from(Job::GenerateThumbnail {
            media_id: "m1".into(),
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
        let result = q.fail("nonexistent", "err").await;
        assert!(result.is_err());
    }
}

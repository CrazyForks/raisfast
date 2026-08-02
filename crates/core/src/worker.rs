//! Background job queue
//!
//! An async task system persisted via `SQLite`, decoupled from the business layer through `EventBus`.
//!
//! Data flow:
//! ```text
//! Service → EventBus.emit(Event) → JobEnqueuer → SQLite jobs table → WorkerRunner → JobHandler
//! ```

mod dispatcher;
mod enqueuer;
mod handler;
pub mod handlers;
mod job_queue;
mod runner;
mod scheduler;

use crate::types::snowflake_id::SnowflakeId;
use serde::{Deserialize, Serialize};

use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

define_enum!(
    JobStatus {
        Pending = "pending",
        Running = "running",
        Completed = "completed",
        Failed = "failed",
        Dead = "dead",
    }
);

define_enum!(
    CronExecStatus {
        Running = "running",
        Completed = "completed",
        Failed = "failed",
    }
);

pub use dispatcher::PluginCronDispatcher;
pub use enqueuer::JobEnqueuer;
pub use handler::{JobHandler, JobHandlerRegistry, LogJobHandler};
pub use job_queue::DefaultJobQueue;
pub use runner::WorkerRunner;
pub use scheduler::{
    CronExecutionLog, CronSchedule, CronScheduler, cleanup_execution_logs, complete_execution_log,
    create_execution_log, create_schedule, create_schedule_with_plugin, delete_schedule,
    fail_execution_log, find_by_id, list_execution_logs, list_schedules, next_run,
    recent_execution_logs, remove_plugin_crons, seed_defaults, sync_plugin_crons, toggle_schedule,
    update_schedule,
};

#[cfg(feature = "export-types")]
export_types!(JobStatus, CronExecStatus, CronSchedule, CronExecutionLog,);

/// Job types and parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Job {
    SendWelcomeEmail {
        user_id: SnowflakeId,
        email: String,
        username: String,
    },
    GenerateThumbnail {
        media_id: SnowflakeId,
        size: u32,
    },
    ScheduledPublish {
        post_id: SnowflakeId,
    },
    WebhookNotify {
        url: String,
        payload: serde_json::Value,
    },
    RebuildSearchIndex {
        post_ids: Vec<i64>,
    },
    InvalidateCache {
        keys: Vec<String>,
    },
    GenerateSitemap,
    SendPasswordResetEmail {
        user_id: SnowflakeId,
        email: String,
        reset_token: String,
    },
    SendSmsCode {
        phone: String,
        code: String,
        purpose: String,
    },
    SendEmailVerification {
        user_id: SnowflakeId,
        email: String,
        verify_token: String,
    },
    ExpirePaymentOrders,
    ExpireOrders,
    RetryPaymentCallback {
        payment_order_id: SnowflakeId,
    },
    ReconcilePayments,
    ProcessWalletOutbox,
    /// Custom job type, supports arbitrary `job_type` + JSON payload
    ///
    /// When no built-in Handler matches, WorkerRunner falls back to plugin dispatch.
    Custom {
        job_type: String,
        payload: serde_json::Value,
    },
}

impl Job {
    /// Returns the `job_type` string (serde tag)
    #[must_use]
    pub fn job_type(&self) -> &str {
        match self {
            Job::SendWelcomeEmail { .. } => "send_welcome_email",
            Job::GenerateThumbnail { .. } => "generate_thumbnail",
            Job::ScheduledPublish { .. } => "scheduled_publish",
            Job::WebhookNotify { .. } => "webhook_notify",
            Job::RebuildSearchIndex { .. } => "rebuild_search_index",
            Job::InvalidateCache { .. } => "invalidate_cache",
            Job::GenerateSitemap => "generate_sitemap",
            Job::SendPasswordResetEmail { .. } => "send_password_reset_email",
            Job::SendSmsCode { .. } => "send_sms_code",
            Job::SendEmailVerification { .. } => "send_email_verification",
            Job::ExpirePaymentOrders => "expire_payment_orders",
            Job::ExpireOrders => "expire_orders",
            Job::RetryPaymentCallback { .. } => "retry_payment_callback",
            Job::ReconcilePayments => "reconcile_payments",
            Job::ProcessWalletOutbox => "process_wallet_outbox",
            Job::Custom { job_type, .. } => job_type,
        }
    }
}

/// Enqueue parameters
#[derive(Debug, Clone)]
pub struct NewJob {
    pub job: Job,
    pub max_attempts: Option<u32>,
    pub run_after: Option<Timestamp>,
}

impl From<Job> for NewJob {
    fn from(job: Job) -> Self {
        Self {
            job,
            max_attempts: None,
            run_after: None,
        }
    }
}

/// Dequeued job record
#[derive(Debug, Clone)]
pub struct QueuedJob {
    pub id: String,
    pub job: Job,
    pub attempts: u32,
    pub max_attempts: u32,
    pub created_at: Timestamp,
}

/// Job queue trait
#[async_trait::async_trait]
pub trait JobQueue: Send + Sync {
    async fn enqueue(&self, new_job: NewJob) -> AppResult<()>;
    async fn dequeue(&self, limit: usize) -> AppResult<Vec<QueuedJob>>;
    async fn complete(&self, id: &str) -> AppResult<()>;
    async fn fail(&self, id: &str, error: &str) -> AppResult<()>;
    async fn dead(&self, id: &str, error: &str) -> AppResult<()>;
    async fn stats(&self) -> AppResult<JobStats>;
    async fn list(
        &self,
        status: Option<JobStatus>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<JobRow>, i64)>;
    async fn retry(&self, id: &str) -> AppResult<()>;
    async fn remove(&self, id: &str) -> AppResult<()>;
    async fn cleanup(&self) -> AppResult<u64>;
}

/// Job statistics
#[derive(Debug, Clone, Default, Serialize)]
pub struct JobStats {
    pub pending: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
    pub dead: i64,
}

/// Job list row (for admin API)
#[derive(Debug, Clone, Serialize)]
pub struct JobRow {
    pub id: String,
    pub job_type: String,
    pub payload: String,
    pub status: JobStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub run_after: Option<Timestamp>,
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Backoff duration: exponential backoff + jitter
#[must_use]
pub fn backoff_duration(attempts: u32) -> std::time::Duration {
    let base_secs: u64 = 10;
    let delay_secs = base_secs.saturating_mul(1u64 << attempts.min(6));
    let jitter = 0.85 + (f64::from(rand_id(attempts)) / f64::from(u32::MAX)) * 0.3;
    std::time::Duration::from_secs_f64(delay_secs as f64 * jitter)
}

/// Deterministic pseudo-random to avoid depending on the rand crate
fn rand_id(seed: u32) -> u32 {
    let mut x = seed.wrapping_mul(1103515245).wrapping_add(12345);
    x = x ^ (x >> 16);
    x = x.wrapping_mul(0x45d9f3b);
    x = x ^ (x >> 16);
    x
}

/// Parses a payload JSON into a Job
///
/// First attempts to match built-in enum variants; on failure, falls back to `Job::Custom`.
fn parse_job(job_type: &str, payload: &str) -> AppResult<Job> {
    let tagged = if payload.is_empty() || payload == "null" {
        format!(r#"{{"type":"{job_type}"}}"#)
    } else {
        format!(r#"{{"type":"{job_type}","payload":{payload}}}"#)
    };

    if let Ok(job) = serde_json::from_str::<Job>(&tagged) {
        return Ok(job);
    }

    let payload_value: serde_json::Value = if payload.is_empty() || payload == "null" {
        serde_json::Value::Null
    } else {
        serde_json::from_str(payload).unwrap_or(serde_json::Value::Null)
    };

    Ok(Job::Custom {
        job_type: job_type.to_string(),
        payload: payload_value,
    })
}

/// Serializes a Job into a payload JSON
fn serialize_job(job: &Job) -> String {
    if let Job::Custom { payload, .. } = job {
        if payload.is_null() {
            "null".to_string()
        } else {
            payload.to_string()
        }
    } else {
        let full = serde_json::to_value(job).unwrap_or_default();
        match full.get("payload") {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_type_matches_tag() {
        let job = Job::GenerateSitemap;
        assert_eq!(job.job_type(), "generate_sitemap");

        let job = Job::SendWelcomeEmail {
            user_id: SnowflakeId(1),
            email: "a@b.com".into(),
            username: "alice".into(),
        };
        assert_eq!(job.job_type(), "send_welcome_email");
    }

    #[test]
    fn serialize_roundtrip() {
        let job = Job::RebuildSearchIndex {
            post_ids: vec![1, 2],
        };
        let payload = serialize_job(&job);
        let parsed = parse_job("rebuild_search_index", &payload).unwrap();
        assert!(matches!(parsed, Job::RebuildSearchIndex { .. }));
    }

    #[test]
    fn serialize_roundtrip_unit_variant() {
        let job = Job::GenerateSitemap;
        let payload = serialize_job(&job);
        let parsed = parse_job("generate_sitemap", &payload).unwrap();
        assert!(matches!(parsed, Job::GenerateSitemap));
    }

    #[test]
    fn backoff_increases() {
        let d0 = backoff_duration(0);
        let d1 = backoff_duration(1);
        let d2 = backoff_duration(2);
        assert!(d1 > d0);
        assert!(d2 > d1);
    }

    #[test]
    fn new_job_from_job() {
        let nj: NewJob = Job::GenerateSitemap.into();
        assert!(nj.max_attempts.is_none());
        assert!(nj.run_after.is_none());
    }

    #[test]
    fn parse_job_fallback_to_custom() {
        let job = parse_job("my_custom_task", r#"{"hello":"world"}"#).unwrap();
        match job {
            Job::Custom { job_type, payload } => {
                assert_eq!(job_type, "my_custom_task");
                assert_eq!(payload["hello"], "world");
            }
            _ => panic!("expected Custom variant"),
        }
    }

    #[test]
    fn parse_job_custom_no_payload() {
        let job = parse_job("simple_task", "").unwrap();
        match job {
            Job::Custom { job_type, payload } => {
                assert_eq!(job_type, "simple_task");
                assert!(payload.is_null());
            }
            _ => panic!("expected Custom variant"),
        }
    }

    #[test]
    fn custom_job_roundtrip() {
        let job = Job::Custom {
            job_type: "my_task".into(),
            payload: serde_json::json!({"key": "value"}),
        };
        assert_eq!(job.job_type(), "my_task");

        let payload = serialize_job(&job);
        let parsed = parse_job("my_task", &payload).unwrap();
        match parsed {
            Job::Custom { job_type, payload } => {
                assert_eq!(job_type, "my_task");
                assert_eq!(payload["key"], "value");
            }
            _ => panic!("expected Custom variant"),
        }
    }
}

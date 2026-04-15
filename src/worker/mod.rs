//! 后台任务队列
//!
//! 基于 `SQLite` 持久化的异步任务系统，通过 `EventBus` 与业务层解耦。
//!
//! 数据流：
//! ```text
//! Service → EventBus.emit(Event) → JobEnqueuer → SQLite jobs 表 → WorkerRunner → JobHandler
//! ```

mod dispatcher;
mod enqueuer;
mod handler;
pub mod handlers;
mod runner;
mod scheduler;
mod sqlite_queue;

use serde::{Deserialize, Serialize};

pub use dispatcher::PluginCronDispatcher;
pub use enqueuer::JobEnqueuer;
pub use handler::{JobHandler, JobHandlerRegistry, LogJobHandler};
pub use runner::WorkerRunner;
pub use scheduler::{
    CronExecutionLog, CronSchedule, CronScheduler, cleanup_execution_logs, complete_execution_log,
    create_execution_log, create_schedule, create_schedule_with_plugin, delete_schedule,
    fail_execution_log, find_by_id, list_execution_logs, list_schedules, next_run,
    recent_execution_logs, remove_plugin_crons, seed_defaults, sync_plugin_crons, toggle_schedule,
    update_schedule,
};
pub use sqlite_queue::SqliteJobQueue;

use crate::errors::app_error::AppResult;

/// 任务类型与参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Job {
    SendWelcomeEmail {
        user_id: String,
        email: String,
        username: String,
    },
    GenerateThumbnail {
        media_id: String,
        size: u32,
    },
    ScheduledPublish {
        post_id: String,
    },
    WebhookNotify {
        url: String,
        payload: serde_json::Value,
    },
    RebuildSearchIndex {
        post_ids: Vec<String>,
    },
    InvalidateCache {
        keys: Vec<String>,
    },
    GenerateSitemap,
    /// 自定义任务类型，支持任意 `job_type` + JSON payload
    ///
    /// 内置 Handler 无法匹配时，WorkerRunner 会 fallback 到插件调度。
    Custom {
        job_type: String,
        payload: serde_json::Value,
    },
}

impl Job {
    /// 返回 `job_type` 字符串（serde tag）
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
            Job::Custom { job_type, .. } => job_type,
        }
    }
}

/// 入队参数
#[derive(Debug, Clone)]
pub struct NewJob {
    pub job: Job,
    pub max_attempts: Option<u32>,
    pub run_after: Option<String>,
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

/// 出队的任务记录
#[derive(Debug, Clone)]
pub struct QueuedJob {
    pub id: String,
    pub job: Job,
    pub attempts: u32,
    pub max_attempts: u32,
    pub created_at: String,
}

/// 任务队列 trait
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
        status: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<JobRow>, i64)>;
    async fn retry(&self, id: &str) -> AppResult<()>;
    async fn remove(&self, id: &str) -> AppResult<()>;
    async fn cleanup(&self) -> AppResult<u64>;
}

/// 任务统计
#[derive(Debug, Clone, Default, Serialize)]
pub struct JobStats {
    pub pending: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
    pub dead: i64,
}

/// 任务列表行（管理 API）
#[derive(Debug, Clone, Serialize)]
pub struct JobRow {
    pub id: String,
    pub job_type: String,
    pub payload: String,
    pub status: String,
    pub attempts: u32,
    pub max_attempts: u32,
    pub run_after: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 退避时间：指数退避 + 抖动
#[must_use]
pub fn backoff_duration(attempts: u32) -> std::time::Duration {
    let base_secs: u64 = 10;
    let delay_secs = base_secs.saturating_mul(1u64 << attempts.min(6));
    let jitter = 0.85 + (f64::from(rand_id(attempts)) / f64::from(u32::MAX)) * 0.3;
    std::time::Duration::from_secs_f64(delay_secs as f64 * jitter)
}

/// 确定性伪随机，避免引入 rand crate
fn rand_id(seed: u32) -> u32 {
    let mut x = seed.wrapping_mul(1103515245).wrapping_add(12345);
    x = x ^ (x >> 16);
    x = x.wrapping_mul(0x45d9f3b);
    x = x ^ (x >> 16);
    x
}

/// 解析 payload JSON 为 Job
///
/// 先尝试匹配内置枚举变体，不匹配则 fallback 为 `Job::Custom`。
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

/// 将 Job 序列化为 payload JSON
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
            user_id: "u1".into(),
            email: "a@b.com".into(),
            username: "alice".into(),
        };
        assert_eq!(job.job_type(), "send_welcome_email");
    }

    #[test]
    fn serialize_roundtrip() {
        let job = Job::RebuildSearchIndex {
            post_ids: vec!["p1".into(), "p2".into()],
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

//! Cron handler: ping — appends a timestamped line to `cron-ping.log`.
//!
//! Zero-dependency demo task for verifying the full cron → job → handler pipeline.
//! Result is immediately visible via `tail -f <storage_root_dir>/cron-ping.log`.

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::errors::app_error::AppResult;
use crate::worker::handler::HandlerMeta;
use crate::worker::{Job, JobHandler};

/// Metadata for the admin task menu.
pub const META: HandlerMeta = HandlerMeta {
    id: "ping",
    display_name: "Ping 测试",
    description: "向 cron-ping.log 追加一行时间戳，用于验证调度链路是否畅通",
    category: "系统维护",
    params_schema: Some(
        r#"{"type":"object","title":"Ping 参数","properties":{"message":{"type":"string","title":"消息内容","default":"hello","description":"写入日志的自定义消息"}}}"#,
    ),
    icon: Some("activity"),
};

/// Ping handler — writes a line to `{storage_root_dir}/cron-ping.log`.
pub struct PingHandler {
    config: Arc<AppConfig>,
}

impl PingHandler {
    /// Creates a new ping handler
    #[must_use]
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl JobHandler for PingHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let message = match job {
            Job::Custom { payload, .. } => payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("hello"),
            _ => "hello",
        };

        let log_path = format!("{}/cron-ping.log", self.config.storage_root_dir);
        let now = crate::utils::tz::now_utc();
        let line = format!("[{now}] ping: {message}\n");

        // Ensure storage dir exists, then append
        if let Err(e) = tokio::fs::create_dir_all(&self.config.storage_root_dir).await {
            tracing::warn!("[ping] failed to create dir: {e}");
        }

        use tokio::io::AsyncWriteExt;
        match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()).await {
                    tracing::error!("[ping] write failed: {e}");
                } else {
                    tracing::info!("[ping] wrote to {log_path}: {message}");
                }
            }
            Err(e) => {
                tracing::error!("[ping] open failed: {e}");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn ping_writes_to_log() {
        let dir = TempDir::new().unwrap();
        let mut config = AppConfig::test_defaults();
        config.storage_root_dir = dir.path().to_str().unwrap().to_string();

        let handler = PingHandler::new(Arc::new(config));
        let job = Job::Custom {
            job_type: "ping".into(),
            payload: serde_json::json!({"message": "test-ping"}),
        };

        handler.handle(&job).await.unwrap();

        let content = tokio::fs::read_to_string(dir.path().join("cron-ping.log"))
            .await
            .unwrap();
        assert!(content.contains("test-ping"));
    }
}

//! `integration.egress_cleanup` — retention cleanup for `itg_egress_log`
//! (same 90d-style policy as receipts, §10.2). Schedule via the admin cron
//! menu; retention comes from `INTEGRATION_EGRESS_LOG_RETENTION_DAYS`.

use crate::errors::app_error::AppResult;
use crate::worker::Job;
use crate::worker::handler::{HandlerMeta, JobHandler};

static META: HandlerMeta = HandlerMeta {
    id: "integration.egress_cleanup",
    display_name: "集成出站日志清理",
    description: "按保留期清理 itg_egress_log 过期行（保留期由 INTEGRATION_EGRESS_LOG_RETENTION_DAYS 配置，默认 90 天）",
    category: "集成",
    params_schema: Some(
        r#"{"type":"object","properties":{"retention_days":{"type":"integer","description":"保留天数（默认取配置 90）"}}}"#,
    ),
    icon: None,
};

pub struct EgressCleanupHandler {
    default_retention_days: u64,
}

impl EgressCleanupHandler {
    #[must_use]
    pub fn new(default_retention_days: u64) -> Self {
        Self {
            default_retention_days,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for EgressCleanupHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };
        let retention = payload
            .get("retention_days")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(self.default_retention_days);
        let Some(plane) = crate::integration::shared() else {
            return Ok(());
        };
        let removed = crate::integration::egress::cleanup_old(plane.pool(), retention).await?;
        if removed > 0 {
            tracing::info!(removed, retention_days = retention, "egress log cleanup");
        }
        Ok(())
    }
}

crate::register_cron_handler!(&META, |deps| Box::new(EgressCleanupHandler::new(
    deps.config.integration.egress_log_retention_days
)));

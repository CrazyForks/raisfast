//! Builtin handler: run a flow's latest published version from a job.
//!
//! Programmatic trigger: `job.enqueue("flow.run", { flow_id, inputs })` from
//! any allowed plugin/pipeline; also selectable from the admin cron task menu
//! (`job_type = "flow_run"`) for scheduled flow executions.
//!
//! Payload:
//! ```json
//! { "flow_id": "<id>", "inputs": { ... } }
//! ```

use std::sync::Arc;

use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::plugins::PluginManager;
use crate::worker::handler::HandlerMeta;
use crate::worker::{Job, JobHandler};

/// Handler that runs a flow from a job payload.
pub struct FlowRunHandler {
    pool: Pool,
    plugins: Arc<PluginManager>,
}

impl FlowRunHandler {
    /// Creates a new handler.
    #[must_use]
    pub fn new(pool: Pool, plugins: Arc<PluginManager>) -> Self {
        Self { pool, plugins }
    }
}

/// Handler meta so the job appears in the admin cron task picker.
pub static META: HandlerMeta = HandlerMeta {
    id: "flow_run",
    display_name: "运行流程",
    description: "执行某个流程的最新已发布版本（可传 inputs）",
    category: "流程",
    params_schema: Some(
        r#"{
  "type": "object",
  "properties": {
    "flow": { "type": "string", "description": "流程唯一名称（可读，默认租户）" },
    "flow_id": { "type": "string", "description": "流程 id（兼容旧调用）" },
    "inputs": { "type": "object", "description": "透传给 start 节点的入参" }
  }
}"#,
    ),
    icon: Some("Workflow"),
};

#[async_trait::async_trait]
impl JobHandler for FlowRunHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };
        let flow_id = if let Some(name) = payload
            .get("flow")
            .and_then(|v| v.as_str())
            .filter(|n| !n.trim().is_empty())
        {
            // Prefer the human-readable unique flow name.
            let flow = crate::flows::model::find_flow_by_name(
                &self.pool,
                crate::constants::DEFAULT_TENANT,
                name.trim(),
            )
            .await?
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "flow_run: no flow named '{name}' in default tenant"
                ))
            })?;
            flow.id
        } else if let Some(raw) = payload.get("flow_id") {
            let id_str = raw
                .as_str()
                .map(str::to_string)
                .or_else(|| raw.as_i64().map(|n| n.to_string()))
                .ok_or_else(|| {
                    AppError::Internal(anyhow::anyhow!("flow_run: invalid 'flow_id'"))
                })?;
            crate::types::snowflake_id::parse_id(&id_str)?
        } else {
            return Err(AppError::Internal(anyhow::anyhow!(
                "flow_run: missing 'flow' (name) or 'flow_id' in payload"
            )));
        };
        let inputs = payload.get("inputs").cloned().filter(|v| !v.is_null());
        crate::flows::run::run_flow_latest(
            &self.pool,
            crate::integration::shared(),
            Some(self.plugins.clone()),
            flow_id,
            inputs,
            "job",
        )
        .await
        .map(|_| ())
    }
}

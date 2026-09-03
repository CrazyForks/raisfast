//! System/basic tools (date, …) available to any agent.

use async_trait::async_trait;
use chrono::Datelike;
use raisfast_agent::tool::ToolExecution;
use raisfast_agent::{Tool, ToolRegistry};
use serde_json::Value;

use crate::AppState;
use crate::middleware::auth::AuthUser;

/// Register system tools onto the shared per-turn registry.
pub fn register(registry: &mut ToolRegistry, _state: &AppState, _auth: &AuthUser) {
    registry.register(TodayTool);
}

// ─────────────────────────────── today ──────────────────────────────────────

struct TodayTool;

#[async_trait]
impl Tool for TodayTool {
    fn name(&self) -> &str {
        "today"
    }

    fn description(&self) -> &str {
        "Return today's UTC date as YYYY-MM-DD."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn execute(&self, _args: Value) -> ToolExecution {
        let now = crate::utils::tz::now_utc();
        Ok(format!(
            "{:04}-{:02}-{:02}",
            now.year(),
            now.month(),
            now.day()
        ))
    }
}

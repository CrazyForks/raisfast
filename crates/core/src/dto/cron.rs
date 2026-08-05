use serde::Deserialize;
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateCronRequest {
    #[validate(length(min = 1, message = "label is required"))]
    pub label: String,
    #[validate(length(min = 1, message = "cron_expr is required"))]
    pub cron_expr: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Execution kind: "builtin" (default), "script", "system", "plugin"
    #[serde(default = "default_builtin")]
    pub exec_kind: String,
    /// Handler ID from the cron task menu (required for builtin)
    pub handler_id: Option<String>,
    /// JSON params validated against the handler's params_schema
    pub params: Option<serde_json::Value>,

    // ── Legacy fields (backward compat) ──────────────────────────────
    /// Legacy: ignored if handler_id is set. Still used for plugin/old schedules.
    pub job_type: Option<String>,
    pub payload: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_builtin() -> String {
    "builtin".into()
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateCronRequest {
    #[validate(length(min = 1, message = "label is required"))]
    pub label: Option<String>,
    pub job_type: Option<String>,
    pub payload: Option<Option<String>>,
    #[validate(length(min = 1, message = "cron_expr is required"))]
    pub cron_expr: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LogQueryParams {
    pub schedule_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct ToggleBody {
    pub enabled: bool,
}

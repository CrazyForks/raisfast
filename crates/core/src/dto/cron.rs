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

    // ── Script fields (exec_kind = "script" or "system") ────────────
    /// Script language: "js", "lua", or "rhai" (script only)
    pub script_lang: Option<String>,
    /// Raw script source code (script) or shell command (system)
    pub script_source: Option<String>,
    /// Entry function name (default "on_cron_tick")
    pub script_entry: Option<String>,

    // ── System fields (exec_kind = "system") ────────────────────────
    /// Whether to wrap command in /bin/sh -c (default true)
    pub use_shell: Option<bool>,
    /// Timeout in seconds (default from config or 300)
    pub timeout_secs: Option<i32>,

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
    #[validate(length(min = 1, message = "cron_expr is required"))]
    pub cron_expr: Option<String>,
    pub enabled: Option<bool>,

    // ── Builtin fields ──────────────────────────────────────────────
    pub exec_kind: Option<String>,
    pub handler_id: Option<String>,
    pub params: Option<serde_json::Value>,

    // ── Script fields ───────────────────────────────────────────────
    pub script_lang: Option<String>,
    pub script_source: Option<String>,
    pub script_entry: Option<String>,

    // ── Legacy ──────────────────────────────────────────────────────
    pub job_type: Option<String>,
    pub payload: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct LogQueryParams {
    pub schedule_id: Option<String>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    /// Legacy: still accepted but ignored if page/page_size present
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct ToggleBody {
    pub enabled: bool,
}

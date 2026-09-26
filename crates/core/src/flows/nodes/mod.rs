//! Node registry + config schemas (contracts.md C1; media-nodes.md for the
//! modality node group).
//!
//! One node = one file: each node's config + executor + node-specific helpers
//! live in its submodule (`chat`/`image`/`speech`/`ct`/`http`/`docparse`);
//! engine-inline nodes (start/end/script/egress/branch/transform/iteration
//! and await's park) keep their configs here. `validate_node(type, version,
//! config)` deserializes config into a strong Rust struct — shape errors
//! surface as 400 here, not at runtime. Unknown keys are tolerated
//! (extra=allow) so frontend can carry display fields; required keys
//! and value shapes are enforced.

pub mod chat;
pub mod ct;
pub mod docparse;
pub mod http;
pub mod image;
pub mod music;
pub mod render;
pub mod speech;
pub mod video;

pub use chat::{ChatConfig, ChatMessage};
pub use ct::{CtConfig, CtFilterRow, CtSetRow};
pub use docparse::DocParseConfig;
pub use http::{HttpConfig, HttpKeyValue};
pub use image::ImageConfig;
pub use music::MusicConfig;
pub use render::RenderConfig;
pub use speech::SpeechConfig;
pub use video::{DEFAULT_VIDEO_DEADLINE_SECS, VideoConfig};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};

/// Control types for start inputs (v2: value-oriented names + five-way file
/// family; `file` is the catch-all "other" bucket).
pub const START_PARAM_TYPES: &[&str] = &[
    "text",
    "paragraph",
    "select",
    "number",
    "boolean",
    "json",
    "file",
    "file-array",
];

/// Allowed file categories for `accept` (no "any" — other is the catch-all).
pub const FILE_ACCEPT_TYPES: &[&str] = &["document", "image", "audio", "video", "other"];

/// File-family control types (`accept` applies to these only).
#[must_use]
pub fn is_file_kind(kind: &str) -> bool {
    matches!(kind, "file" | "file-array")
}

/// Reserved handle names (contracts.md C1.3).
pub const H_IN: &str = "in";
pub const H_OUT: &str = "out";
pub const H_ERROR_OUT: &str = "error_out";

/// Await approval-kind output handles (await-node.md §4; Dify UserAction
/// id=output-handle). Human action ports come from `config.actions` (any id);
/// `timeout` is the built-in extra port when `timeout_secs` is set.
pub const H_TIMEOUT: &str = "timeout";

/// Default action when no `actions` are declared (Dify's
/// `_DEFAULT_SUBMIT_ACTION = {id: "submit", title: "Submit"}`).
pub const AWAIT_DEFAULT_ACTION: &str = "submit";

/// Human input field types (Dify FormInputConfig subset).
pub const AWAIT_INPUT_TYPES: &[&str] = &["string", "number", "boolean", "select"];

/// Action id / title limits (Dify `_IDENTIFIER_PATTERN` max 20; title ≤ 100).
pub const AWAIT_ACTION_ID_MAX: usize = 20;
pub const AWAIT_ACTION_TITLE_MAX: usize = 100;

/// `action` id must match (Dify's identifier rule): letter/underscore start.
#[must_use]
pub fn is_valid_action_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= AWAIT_ACTION_ID_MAX
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Known node types. `chat` was renamed from `llm` (media-nodes.md §1):
/// node types align with the llm-facade modality names. The legacy `"llm"`
/// wire value is accepted as a permanent read alias (graph.rs `read_node`)
/// so pre-migration canvas JSON keeps running.
pub const T_START: &str = "start";
pub const T_END: &str = "end";
pub const T_SCRIPT: &str = "script";
pub const T_EGRESS: &str = "egress";
pub const T_BRANCH: &str = "branch";
pub const T_AWAIT: &str = "await";
pub const T_TRANSFORM: &str = "transform";
pub const T_CHAT: &str = "chat";
/// Legacy alias of [`T_CHAT`] written by pre-rename canvases; read-only.
pub const T_LLM_LEGACY: &str = "llm";
pub const T_IMAGE: &str = "image";
pub const T_SPEECH: &str = "speech";
pub const T_MUSIC: &str = "music";
pub const T_RENDER: &str = "render";
pub const T_VIDEO: &str = "video";
pub const T_HTTP: &str = "http";
pub const T_CT: &str = "ct";
pub const T_ITERATION: &str = "iteration";
pub const T_DOCPARSE: &str = "docparse";

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct StartParam {
    /// Variable key of the start namespace (v2: was `name` — one word, one job).
    pub variable: String,
    #[serde(default)]
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub default: Option<Value>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub max_length: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub options: Option<Vec<Value>>,
    /// File-category constraint, required for `file` / `file-array` params:
    /// a non-empty subset of document|image|audio|video|other — multi-select
    /// (e.g. documents AND images) is allowed (v2 — no "any" bucket).
    #[serde(default)]
    pub accept: Option<Vec<String>>,
    /// Max number of files; `file-array` only, integer ≥ 1 when present.
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub max_count: Option<i64>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct StartConfig {
    #[serde(default)]
    pub params: Vec<StartParam>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct EndOutput {
    /// Result key in the final run output (v2: was `name`).
    pub key: String,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub value: Value,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct EndConfig {
    #[serde(default)]
    pub outputs: Vec<EndOutput>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct SandboxLimits {
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_ms: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub memory_mb: Option<i64>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct HostPermissions {
    /// Outbound api-client keys the script may call via `egress.call` / `host.callApi`.
    #[serde(default)]
    pub call_api: Option<Vec<String>>,
    /// Content types (plural names) the script may touch via `ct.*` host APIs
    /// (`*` = all; empty/absent = denied).
    #[serde(default)]
    pub content_types: Option<Vec<String>>,
    /// Raw SQL tables (read-only / read-write forms) via the `db` host API.
    #[serde(default)]
    pub database: Option<Vec<String>>,
    /// Raw HTTP domain whitelist (`*.example.com`, `api.example.com/*`).
    #[serde(default)]
    pub http: Option<Vec<String>>,
    /// Session-token actions (`issue`/`verify`).
    #[serde(default)]
    pub session: Option<Vec<String>>,
    /// Presence actions (`available`/`status`/`report`).
    #[serde(default)]
    pub presence: Option<Vec<String>>,
    #[serde(default)]
    pub data: Option<bool>,
    #[serde(default)]
    pub emit: Option<bool>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptConfig {
    pub language: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub fn_name: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub input: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub output_schema: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub sandbox: Option<SandboxLimits>,
    #[serde(default)]
    pub host_permissions: Option<HostPermissions>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct EgressConfig {
    pub client_key: String,
    #[serde(default)]
    pub op: String,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub input: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub output_schema: Option<serde_json::Map<String, Value>>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct BranchRule {
    #[serde(default)]
    pub label: String,
    /// Structured condition or expression string.
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub when: Value,
    #[serde(default)]
    pub handle: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct BranchConfig {
    #[serde(default)]
    pub branches: Vec<BranchRule>,
    #[serde(default)]
    pub else_handle: Option<String>,
}

/// `transform` node assignment (transform-node.md §2.1): `key` becomes a
/// namespace field of the node; `value` is a full ValueExpr (literal/ref/
/// expr with the cleaning-function set).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct TransformAssignment {
    pub key: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub value: Value,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct TransformConfig {
    #[serde(default)]
    pub assignments: Vec<TransformAssignment>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct AwaitConfig {
    /// Display text shown to the actor at resume time — a `{{#…#}}`
    /// template interpolated against the run pool before the parked state
    /// surfaces (Dify `HumanInputNodeData.form_content`).
    #[serde(default)]
    pub form_content: String,
    /// Data-collection fields (Dify `FormInputConfig`, scoped to
    /// string/number/boolean/select with constant options).
    #[serde(default)]
    pub inputs: Vec<AwaitInputField>,
    /// Custom action buttons (Dify `UserActionConfig`): each `id` IS an
    /// output handle the author wires; `title` is the button text. Empty
    /// actions default to a single `submit` action (Dify's default).
    #[serde(default)]
    pub actions: Vec<AwaitAction>,
    /// Soft routing only (task list / notification audience) — NOT an authz
    /// boundary (await-node.md §6). Admin auth guards the resume endpoint;
    /// the public resume token guards the callback endpoint.
    #[serde(default)]
    pub approvers: Vec<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_secs: Option<i64>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct AwaitInputField {
    pub name: String,
    #[serde(default)]
    pub label: String,
    /// string | number | boolean | select
    #[serde(default = "default_input_type")]
    pub r#type: String,
    #[serde(default)]
    pub required: bool,
    /// select options (constant; `type=select` requires non-empty).
    #[serde(default)]
    pub options: Vec<String>,
}

fn default_input_type() -> String {
    "string".into()
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct AwaitAction {
    /// Identifier + output handle (Dify: id "also serves as the identifiers
    /// of output handle"). Must satisfy `H_IDENT_MAX`-length identifier rules.
    pub id: String,
    /// Button text shown to the actor (≤ `TITLE_MAX` chars, like Dify).
    #[serde(default)]
    pub title: String,
}

impl AwaitConfig {
    /// Effective actions with the Dify default injected when none declared.
    #[must_use]
    pub fn effective_actions(&self) -> Vec<AwaitAction> {
        if self.actions.is_empty() {
            vec![AwaitAction {
                id: AWAIT_DEFAULT_ACTION.into(),
                title: "Submit".into(),
            }]
        } else {
            self.actions.clone()
        }
    }
}

/// Resume request body (await-node.md §2.2): which action fired plus any
/// collected input data. Trigger-agnostic — admin UI, external systems
/// (public token endpoint) and the timeout sweeper all funnel into the same
/// shape. The await node never knows or cares WHO resumed it.
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResumeEnvelope {
    /// Which action fired — routes through its output handle.
    pub action: String,
    /// The typed input values collected from the actor.
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub data: Option<Value>,
}

impl ResumeEnvelope {
    /// Field-level validation (non-empty action).
    ///
    /// # Errors
    ///
    /// `BadRequest` when `action` is empty.
    pub fn validate(&self) -> crate::errors::app_error::AppResult<()> {
        if self.action.trim().is_empty() {
            return Err(crate::errors::app_error::AppError::BadRequest(
                "resume: action 不能为空".into(),
            ));
        }
        Ok(())
    }

    /// Normalize into `(pool payload, output handle)` (await-node.md §2.3/§4):
    /// the fired action routes through its handle with the collected data
    /// landing under `resume`.
    #[must_use]
    pub fn normalize(&self) -> (Value, Option<String>) {
        (
            self.data.clone().unwrap_or(Value::Null),
            Some(self.action.clone()),
        )
    }
}

/// `iteration` node config (iteration-node.md §1). `body` is a graph
/// definition ({nodes, edges}) isomorphic to the main graph: exactly one
/// start (no params) and exactly one end (single collection point).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct IterationConfig {
    /// ValueExpr resolving to the array to iterate (usually `{"ref": [...]}`).
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub items: Value,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub concurrency: Option<i64>,
    #[serde(default)]
    pub on_item_error: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub max_items: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub body: Option<Value>,
}

/// Item-error semantics: `abort` (default) fails the node; `skip` records
/// `{ok:false, index}` and continues.
pub const ITER_ITEM_ERRORS: &[&str] = &["abort", "skip"];

/// Hard cap on items per run (iteration-node.md §1).
pub const ITER_MAX_ITEMS: i64 = 5000;

/// Max parallel bodies.
pub const ITER_MAX_CONCURRENCY: i64 = 20;

/// Max iteration nesting depth (self included; iteration-node.md §6).
pub const ITER_MAX_DEPTH: usize = 3;

/// Static nesting depth of iteration nodes inside a body config (recursive).
/// Sequential iterations on the same graph do NOT nest — only bodies count.
fn iteration_body_depth(body: &Value) -> usize {
    body.get("nodes")
        .and_then(Value::as_array)
        .map(|ns| {
            ns.iter()
                .filter_map(|n| {
                    let kind = n.get("data")?.get("type")?.as_str()?;
                    if kind != "iteration" {
                        return None;
                    }
                    let cfg = n.get("data")?.get("config")?;
                    Some(1 + iteration_body_depth(cfg.get("body").unwrap_or(&Value::Null)))
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// Declared output fields of a node (v2 D2): what the node writes into its
/// pool namespace. Drives skip-null semantics (D6) and lint law 3 (D4).
///
/// - `start`  → declared start variables
/// - `script` → `output_schema` properties (empty when undeclared → law-3 skip)
/// - `egress` → fixed `response`
/// - `chat`   → fixed `text`/`structured`/`usage`/`latency_ms`
/// - `image`  → fixed `images`/`model`/`n`
/// - `speech` → fixed `audio`/`chars`/`voice`/`model`
/// - `video`  → fixed `resume`（resume payload: `{video:{key,url},task_id,status}`）
/// - `docparse` → fixed `markdown`/`images`/`engine`/`pages`/`warnings`
/// - `await`  → fixed `resume`
/// - `branch` → fixed `handle`
#[must_use]
pub fn declared_output_fields(kind: &str, config: &Value) -> Vec<String> {
    match kind {
        T_START => {
            let Ok(c) = serde_json::from_value::<StartConfig>(config.clone()) else {
                return Vec::new();
            };
            c.params.iter().map(|p| p.variable.clone()).collect()
        }
        T_SCRIPT => {
            let Ok(c) = serde_json::from_value::<ScriptConfig>(config.clone()) else {
                return Vec::new();
            };
            c.output_schema
                .as_ref()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default()
        }
        T_EGRESS => vec!["response".into()],
        T_CHAT => vec![
            "text".into(),
            "structured".into(),
            "usage".into(),
            "latency_ms".into(),
        ],
        T_IMAGE => vec!["images".into(), "model".into(), "n".into()],
        T_SPEECH => vec![
            "audio".into(),
            "chars".into(),
            "voice".into(),
            "model".into(),
        ],
        T_AWAIT => vec!["resume".into()],
        T_HTTP => vec![
            "status".into(),
            "json".into(),
            "body".into(),
            "latency_ms".into(),
        ],
        T_ITERATION => vec!["items".into(), "count".into()],
        T_CT => {
            let op = config.get("op").and_then(Value::as_str).unwrap_or("");
            match op {
                "find_one" | "insert" | "update" => vec!["record".into()],
                "find_page" => vec!["items".into(), "total".into()],
                "count" => vec!["total".into()],
                "delete" => vec!["affected".into()],
                _ => Vec::new(),
            }
        }
        T_BRANCH => vec!["handle".into()],
        T_TRANSFORM => {
            let Ok(c) = serde_json::from_value::<TransformConfig>(config.clone()) else {
                return Vec::new();
            };
            c.assignments.into_iter().map(|a| a.key).collect()
        }
        T_DOCPARSE => vec![
            "markdown".into(),
            "images".into(),
            "engine".into(),
            "pages".into(),
            "warnings".into(),
        ],
        _ => Vec::new(),
    }
}

/// Shallow JSON-Schema check shared by script output validation and the chat
/// structured output path: only `type` and `required` (recursive for nested
/// objects); other keywords ignored (v2 D8 — one validator, no second dialect).
pub fn shallow_schema_check(value: &Value, schema: &Value) -> Result<(), String> {
    if let Some(want) = schema.get("type").and_then(Value::as_str) {
        let ok = match (want, value) {
            ("object", Value::Object(_))
            | ("array", Value::Array(_))
            | ("string", Value::String(_))
            | ("boolean", Value::Bool(_))
            | ("null", Value::Null) => true,
            ("number", Value::Number(_)) => true,
            ("integer", Value::Number(n)) => n.is_i64() || n.is_u64(),
            _ => false,
        };
        if !ok {
            return Err(format!("类型不匹配: 期望 {want}"));
        }
    }
    if let (Value::Object(map), Some(required)) =
        (value, schema.get("required").and_then(Value::as_array))
    {
        for key in required {
            if let Some(k) = key.as_str()
                && !map.contains_key(k)
            {
                return Err(format!("缺少必填字段: {k}"));
            }
        }
    }
    if let (Value::Object(map), Some(props)) =
        (value, schema.get("properties").and_then(Value::as_object))
    {
        for (k, sub) in props {
            if let Some(v) = map.get(k)
                && let Err(e) = shallow_schema_check(v, sub)
            {
                return Err(format!("字段 {k}: {e}"));
            }
        }
    }
    Ok(())
}

/// Validate an input value: scalar (literal shorthand) OR exactly one of
/// `{literal|ref|expr}`. `ref` must be an array of strings.
pub fn validate_value_expr(where_: &str, v: &Value) -> AppResult<()> {
    if v.is_object() {
        let obj = v.as_object().unwrap();
        let keys: Vec<&String> = obj.keys().collect();
        let variant = keys
            .iter()
            .find(|k| k.as_str() == "literal" || k.as_str() == "ref" || k.as_str() == "expr");
        if let Some(k) = variant {
            if keys.len() > 1 {
                return Err(AppError::BadRequest(format!(
                    "{where_}: ValueExpr {v} 只能含一个键 (literal|ref|expr)"
                )));
            }
            if k.as_str() == "ref" {
                let arr = obj.get("ref").and_then(Value::as_array).ok_or_else(|| {
                    AppError::BadRequest(format!("{where_}: ref 必须是字符串数组"))
                })?;
                if arr.iter().any(|s| !s.is_string()) {
                    return Err(AppError::BadRequest(format!(
                        "{where_}: ref 元素必须是字符串"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_input_map(
    where_: &str,
    input: Option<&serde_json::Map<String, Value>>,
) -> AppResult<()> {
    if let Some(map) = input {
        for (k, v) in map {
            validate_value_expr(&format!("{where_}.input.{k}"), v)?;
        }
    }
    Ok(())
}

/// Deserialize + validate a node config against its known schema. Unknown type
/// or version → `BadRequest`. Engine-inline node bodies live here; the
/// submodule-backed nodes (`chat`/`image`/`speech`/`ct`/`http`/`docparse`)
/// validate in their own files (one node = one file, media-nodes.md §5).
pub fn validate_node(kind: &str, _version: i64, config: &Value) -> AppResult<()> {
    let type_error =
        |e: serde_json::Error| AppError::BadRequest(format!("node '{kind}' config invalid: {e}"));
    match kind {
        T_START => {
            let c: StartConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            for p in &c.params {
                if p.variable.is_empty() {
                    return Err(AppError::BadRequest(
                        "start.params[].variable 不能为空".into(),
                    ));
                }
                if p.max_length.is_some_and(|ml| ml < 1) {
                    return Err(AppError::BadRequest(format!(
                        "start.params['{}'].max_length 须为 ≥1 的整数",
                        p.variable
                    )));
                }
                if !START_PARAM_TYPES.contains(&p.kind.as_str()) {
                    return Err(AppError::BadRequest(format!(
                        "start.params['{}'].type '{}' 非法（允许: {}）",
                        p.variable,
                        p.kind,
                        START_PARAM_TYPES.join(" | ")
                    )));
                }
                if is_file_kind(&p.kind) {
                    let accepts = p.accept.as_deref().unwrap_or(&[]);
                    let valid = !accepts.is_empty()
                        && accepts
                            .iter()
                            .all(|a| FILE_ACCEPT_TYPES.contains(&a.as_str()));
                    if !valid {
                        return Err(AppError::BadRequest(format!(
                            "start.params['{}'].accept 须为 {} 的非空子集（可多选，不允许任意文件）",
                            p.variable,
                            FILE_ACCEPT_TYPES.join(" | ")
                        )));
                    }
                } else if p.accept.is_some() {
                    return Err(AppError::BadRequest(format!(
                        "start.params['{}'].accept 仅用于 file/file-array 类型",
                        p.variable
                    )));
                }
                if p.max_count.is_some() && p.kind != "file-array" {
                    return Err(AppError::BadRequest(format!(
                        "start.params['{}'].max_count 仅用于 file-array 类型",
                        p.variable
                    )));
                }
                if p.max_count.is_some_and(|c| c < 1) {
                    return Err(AppError::BadRequest(format!(
                        "start.params['{}'].max_count 须为 ≥1 的整数",
                        p.variable
                    )));
                }
            }
        }
        T_END => {
            let _c: EndConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
        }
        T_SCRIPT => {
            let c: ScriptConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            if c.code.is_empty() && c.plugin_id.is_none() {
                return Err(AppError::BadRequest(
                    "script: code 与 plugin_id 至少给一个".into(),
                ));
            }
            validate_input_map("script", c.input.as_ref())?;
        }
        T_EGRESS => {
            let c: EgressConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            if c.client_key.is_empty() || c.op.is_empty() {
                return Err(AppError::BadRequest("egress: client_key 与 op 必填".into()));
            }
            validate_input_map("egress", c.input.as_ref())?;
        }
        T_BRANCH => {
            let c: BranchConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            if c.branches.is_empty() {
                return Err(AppError::BadRequest("branch: 至少一个 branches".into()));
            }
        }
        T_TRANSFORM => {
            let c: TransformConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            if c.assignments.is_empty() {
                return Err(AppError::BadRequest(
                    "transform: 至少一条 assignments".into(),
                ));
            }
            let mut keys = std::collections::HashSet::new();
            for a in &c.assignments {
                if !is_valid_action_id(&a.key) {
                    return Err(AppError::BadRequest(format!(
                        "transform: key '{}' 非法（字母/下划线开头，≤20 字符）",
                        a.key
                    )));
                }
                if !keys.insert(a.key.clone()) {
                    return Err(AppError::BadRequest(format!(
                        "transform: key 重复: {}",
                        a.key
                    )));
                }
                // ValueExpr shape sanity: ref must be a string array; expr a
                // string (full lint of ref existence runs in lint_graph).
                if let Some(arr) = a.value.get("ref") {
                    let ok = arr
                        .as_array()
                        .is_some_and(|xs| xs.iter().all(Value::is_string));
                    if !ok {
                        return Err(AppError::BadRequest(format!(
                            "transform: assignments['{}'].value.ref 须为字符串数组",
                            a.key
                        )));
                    }
                } else if a.value.get("expr").is_some()
                    && !a.value.get("expr").is_some_and(|v| v.is_string())
                {
                    return Err(AppError::BadRequest(format!(
                        "transform: assignments['{}'].value.expr 须为字符串",
                        a.key
                    )));
                }
            }
        }
        T_AWAIT => {
            let c: AwaitConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            if c.timeout_secs.is_some_and(|t| t < 1) {
                return Err(AppError::BadRequest(
                    "await: timeout_secs 须为 ≥1 的整数".into(),
                ));
            }
            // Actions: valid unique ids; title length (Dify UserActionConfig).
            let mut ids = std::collections::HashSet::new();
            for a in &c.actions {
                if !is_valid_action_id(&a.id) {
                    return Err(AppError::BadRequest(format!(
                        "await: action id '{}' 非法（字母/下划线开头，≤{} 字符）",
                        a.id, AWAIT_ACTION_ID_MAX
                    )));
                }
                if a.id == H_TIMEOUT {
                    return Err(AppError::BadRequest(format!(
                        "await: action id 不能用保留名 '{H_TIMEOUT}'（与内置超时端口冲突）"
                    )));
                }
                if !ids.insert(a.id.clone()) {
                    return Err(AppError::BadRequest(format!(
                        "await: action id 重复: {}",
                        a.id
                    )));
                }
                if a.title.chars().count() > AWAIT_ACTION_TITLE_MAX {
                    return Err(AppError::BadRequest(format!(
                        "await: action '{}' 的 title 超 {} 字符",
                        a.id, AWAIT_ACTION_TITLE_MAX
                    )));
                }
            }
            // Inputs: unique non-empty names; type vocabulary; select
            // requires options (Dify's duplicated output_variable_name
            // validator + FormInputType subset).
            let mut names = std::collections::HashSet::new();
            for f in &c.inputs {
                if f.name.trim().is_empty() || !names.insert(f.name.clone()) {
                    return Err(AppError::BadRequest(format!(
                        "await: inputs 名称重复或为空: {}",
                        f.name
                    )));
                }
                if !AWAIT_INPUT_TYPES.contains(&f.r#type.as_str()) {
                    return Err(AppError::BadRequest(format!(
                        "await: inputs['{}'].type '{}' 非法（{}）",
                        f.name,
                        f.r#type,
                        AWAIT_INPUT_TYPES.join(" | ")
                    )));
                }
                if f.r#type == "select" && f.options.is_empty() {
                    return Err(AppError::BadRequest(format!(
                        "await: inputs['{}'] type=select 须配置非空 options",
                        f.name
                    )));
                }
            }
        }
        T_ITERATION => {
            let c: IterationConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            if c.items.get("ref").is_none() && c.items.get("literal").is_none() {
                return Err(AppError::BadRequest(
                    "iteration: items 须为 ValueExpr（ref 或 literal）".into(),
                ));
            }
            if let Some(e) = &c.on_item_error
                && !ITER_ITEM_ERRORS.contains(&e.as_str())
            {
                return Err(AppError::BadRequest(format!(
                    "iteration: on_item_error '{}' 非法（{}）",
                    e,
                    ITER_ITEM_ERRORS.join("/")
                )));
            }
            if c.concurrency
                .is_some_and(|v| !(1..=ITER_MAX_CONCURRENCY).contains(&v))
            {
                return Err(AppError::BadRequest(format!(
                    "iteration: concurrency 须在 1..={ITER_MAX_CONCURRENCY}"
                )));
            }
            if c.max_items
                .is_some_and(|v| !(1..=ITER_MAX_ITEMS).contains(&v))
            {
                return Err(AppError::BadRequest(format!(
                    "iteration: max_items 须在 1..={ITER_MAX_ITEMS}"
                )));
            }
            // Body structural checks: exactly one start (no params) + one end.
            // Reference lint with the outer-ancestor exception runs in
            // lint_graph (iteration-node.md §6).
            let Some(body) = &c.body else {
                return Err(AppError::BadRequest("iteration: body 不能为空".into()));
            };
            let count_kind = |kind: &str| -> usize {
                body.get("nodes")
                    .and_then(Value::as_array)
                    .map(|ns| {
                        ns.iter()
                            .filter(|n| {
                                n.get("data")
                                    .and_then(|d| d.get("type"))
                                    .and_then(Value::as_str)
                                    == Some(kind)
                            })
                            .count()
                    })
                    .unwrap_or(0)
            };
            let video_ids: Vec<String> = body
                .get("nodes")
                .and_then(Value::as_array)
                .map(|ns| {
                    ns.iter()
                        .filter(|n| {
                            n.get("data")
                                .and_then(|d| d.get("type"))
                                .and_then(Value::as_str)
                                == Some(T_VIDEO)
                        })
                        .filter_map(|n| n.get("id").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            // Video-body iteration (iteration-video.md §2.1): the video node
            // is the item boundary — `end` is optional and video nodes must
            // have no outgoing edges.
            if video_ids.is_empty() && (count_kind("start") != 1 || count_kind("end") != 1) {
                return Err(AppError::BadRequest(
                    "iteration: body 必须恰好一个 start（无参数）和一个 end".into(),
                ));
            }
            if !video_ids.is_empty()
                && let Some(edges) = body.get("edges").and_then(Value::as_array)
            {
                for e in edges {
                    let src = e.get("source").and_then(Value::as_str).unwrap_or_default();
                    if video_ids.iter().any(|v| v == src) {
                        return Err(AppError::BadRequest(
                            "iteration: video 须为 item 的末位产出节点（不得有出边，结果由聚合器回填）"
                                .into(),
                        ));
                    }
                }
            }
            if body
                .get("nodes")
                .and_then(Value::as_array)
                .is_none_or(|ns| ns.is_empty())
            {
                return Err(AppError::BadRequest(
                    "iteration: body.nodes 不能为空".into(),
                ));
            }
            // Nesting depth (self + bodies) — static, publish-time.
            if 1 + iteration_body_depth(body) > ITER_MAX_DEPTH {
                return Err(AppError::BadRequest(format!(
                    "iteration: 嵌套深度超上限 {ITER_MAX_DEPTH}"
                )));
            }
            // await inside a body cannot park (NoopPersist; the snapshot's
            // waiting list is top-level only). Video inside a body IS now
            // supported (submit-only recorder + iteration park, media-nodes.md
            // §8-2 / iteration-video.md §2).
            if body
                .get("nodes")
                .and_then(Value::as_array)
                .is_some_and(|ns| {
                    ns.iter().any(|n| {
                        n.get("data")
                            .and_then(|d| d.get("type"))
                            .and_then(Value::as_str)
                            == Some("await")
                    })
                })
            {
                return Err(AppError::BadRequest(
                    "iteration: 循环体内暂不支持 await（等待语义需外层配合，见 iteration-node.md）"
                        .into(),
                ));
            }
        }
        T_CT => ct::validate(config)?,
        T_HTTP => http::validate(config)?,
        T_CHAT => chat::validate(config)?,
        T_DOCPARSE => docparse::validate(config)?,
        T_IMAGE => image::validate(config)?,
        T_SPEECH => speech::validate(config)?,
        T_MUSIC => music::validate(config)?,
        T_RENDER => render::validate(config)?,
        T_VIDEO => video::validate(config)?,
        other => {
            return Err(AppError::BadRequest(format!(
                "node type '{other}' not supported (start|end|script|egress|branch|await|transform|chat|image|speech|video|music|render|http|ct|iteration|docparse)"
            )));
        }
    }
    Ok(())
}

/// Node type string union (TS literal union; wire = `data.type`). `chat` is
/// the renamed `llm` (media-nodes.md §1) — legacy `"llm"` canvases are
/// aliased at load time (graph.rs `read_node`), never re-saved.
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-types", ts(rename_all = "lowercase"))]
#[allow(dead_code)]
pub enum NodeKind {
    Start,
    End,
    Script,
    Egress,
    Branch,
    Await,
    Chat,
    Image,
    Speech,
    Video,
    Music,
    Render,
    Http,
    Ct,
    Iteration,
    Docparse,
}

/// TS-only union of every node's config shape (editor drives panels off it).
/// Discriminator lives at `node.data.type`; config payloads are the inner
/// object (no tag inside).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-types", ts(untagged))]
#[allow(dead_code)]
#[allow(clippy::large_enum_variant)]
pub enum NodeConfigVariant {
    Start(StartConfig),
    End(EndConfig),
    Script(ScriptConfig),
    Egress(EgressConfig),
    Branch(BranchConfig),
    Await(AwaitConfig),
    Chat(ChatConfig),
    Image(ImageConfig),
    Speech(SpeechConfig),
    Video(VideoConfig),
    Music(MusicConfig),
    Render(RenderConfig),
    Http(HttpConfig),
    Ct(CtConfig),
    Iteration(IterationConfig),
    DocParse(DocParseConfig),
}

/// TS-only union for ValueExpr (literal | ref selector | expr string).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-types", ts(untagged))]
#[allow(dead_code)]
pub enum ValueExpr {
    Literal {
        #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
        literal: Value,
    },
    Ref {
        #[cfg_attr(feature = "export-types", ts(rename = "ref"))]
        ref_: Vec<String>,
    },
    Expr {
        expr: String,
    },
}

// ── shared plumbing for the modality node group (media-nodes.md §5) ──────

/// LLM 底座 runtime shared by the modality nodes (`chat`/`image`/`speech`,
/// later `video`). 模型访问唯一入口 = llm 底座（design §10.2）——节点只带
/// 租户与（可选）触发用户，路由/号池/failover/计费全在内核。
#[derive(Clone)]
pub struct LlmRuntime {
    pub router: std::sync::Arc<crate::llm::service::LlmRouter>,
    pub tenant: String,
    /// 触发用户（计费归因/日限额）；cron/system = None。
    pub caller: Option<crate::types::snowflake_id::SnowflakeId>,
}

/// Render a C3.1 template string against the pool. Prompts are always text:
/// a whole-string `{{#ref#}}` returning an object is stringified instead of
/// failing (C3.1 keeps typed values).
pub(crate) fn render_prompt_text(text: &str, pool: &super::engine::Pool) -> AppResult<String> {
    match super::expr::resolve_text(text, pool)? {
        Value::String(s) => Ok(s),
        other => Ok(match &other {
            Value::String(s) => s.clone(),
            v => serde_json::to_string(v).unwrap_or_default(),
        }),
    }
}

/// Cap for a single media asset download (URL-only upstream images).
pub(crate) const MEDIA_DOWNLOAD_MAX_BYTES: usize = 64 * 1024 * 1024;

/// SSRF-checked HTTPS download of a URL-only upstream asset (media-nodes.md
/// §6 — upstream URLs are transient signed addresses, never forwarded raw).
/// Redirects disabled: a 302 to a private host must not bypass the SSRF check
/// (same discipline as docparse's input loader).
pub(crate) async fn download_https(url: &str) -> AppResult<Vec<u8>> {
    crate::docparse::validate_external_url(url)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("media http client: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("media: 下载失败 {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "media: 下载失败 HTTP {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::BadRequest(format!("media: 读取响应失败 {e}")))?;
    if bytes.len() > MEDIA_DOWNLOAD_MAX_BYTES {
        return Err(AppError::BadRequest(format!(
            "media: 文件过大 {} bytes (max {MEDIA_DOWNLOAD_MAX_BYTES})",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

/// Sniff an image's extension + content type from magic bytes (fallback png).
#[must_use]
pub(crate) fn sniff_image(bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ("png", "image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("jpg", "image/jpeg")
    } else if bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
        ("webp", "image/webp")
    } else if bytes.starts_with(b"GIF8") {
        ("gif", "image/gif")
    } else {
        ("png", "image/png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_known_types_and_rejects_unknown() {
        assert!(validate_node(T_SCRIPT, 1, &json!({"language": "js", "code": "return 1"})).is_ok());
        assert!(
            validate_node(T_SCRIPT, 1, &json!({"language": "js"})).is_err(),
            "无 code/plugin"
        );
        assert!(validate_node(T_EGRESS, 1, &json!({"client_key": "llm", "op": "chat"})).is_ok());
        assert!(validate_node("nope", 1, &json!({})).is_err(), "未知 type");
    }

    #[test]
    fn value_expr_shapes() {
        assert!(validate_value_expr("x", &json!({"ref": ["start", "msg"]})).is_ok());
        assert!(validate_value_expr("x", &json!({"literal": 5})).is_ok());
        assert!(validate_value_expr("x", &json!({"expr": "{{#start.msg#}}.length > 0"})).is_ok());
        assert!(validate_value_expr("x", &json!({"ref": "start"})).is_err());
        assert!(
            validate_value_expr("x", &json!({"ref": ["a"], "literal": 1})).is_err(),
            "多键"
        );
    }

    #[test]
    fn start_param_max_length_bounds() {
        let ok = json!({
            "params": [
                {"variable": "q", "label": "Q", "type": "text", "required": true, "max_length": 100}
            ]
        });
        assert!(validate_node(T_START, 1, &ok).is_ok());

        for bad in [0, -5] {
            let cfg = json!({
                "params": [
                    {"variable": "q", "label": "Q", "type": "text", "max_length": bad}
                ]
            });
            let err = validate_node(T_START, 1, &cfg).unwrap_err();
            assert!(err.to_string().contains("max_length"), "{err}");
        }

        // fractional input is rejected at deserialization (i64)
        let frac = json!({
            "params": [
                {"variable": "q", "label": "Q", "type": "text", "max_length": 1.5}
            ]
        });
        assert!(validate_node(T_START, 1, &frac).is_err());
    }

    #[test]
    fn start_param_file_accept_rules() {
        // file without accept -> rejected (no "any" files)
        let bad = json!({
            "params": [{"variable": "f", "label": "F", "type": "file", "required": true}]
        });
        let err = validate_node(T_START, 1, &bad).unwrap_err();
        assert!(err.to_string().contains("accept"), "{err}");

        // file-array + multi-category accept -> ok
        let ok = json!({
            "params": [
                {"variable": "imgs", "label": "Images", "type": "file-array", "accept": ["image", "document"]}
            ]
        });
        assert!(validate_node(T_START, 1, &ok).is_ok());

        // empty accept array -> rejected
        let empty = json!({
            "params": [{"variable": "f", "label": "F", "type": "file", "accept": []}]
        });
        assert!(validate_node(T_START, 1, &empty).is_err());

        // accept outside the 5 categories -> rejected
        let bad2 = json!({
            "params": [{"variable": "f", "label": "F", "type": "file", "accept": ["anything"]}]
        });
        assert!(validate_node(T_START, 1, &bad2).is_err());

        // accept on non-file type -> rejected
        let bad3 = json!({
            "params": [{"variable": "q", "label": "Q", "type": "text", "accept": "image"}]
        });
        assert!(validate_node(T_START, 1, &bad3).is_err());

        // legacy control type renamed away -> rejected
        let bad4 = json!({
            "params": [{"variable": "q", "label": "Q", "type": "text-input"}]
        });
        assert!(validate_node(T_START, 1, &bad4).is_err());
    }

    fn iter_body_with(inner_body: Value) -> Value {
        json!({
            "nodes": [
                {"id": "bstart", "data": {"type": "start", "config": {}}},
                {"id": "it", "data": {"type": "iteration", "config": {
                    "items": {"literal": [1]},
                    "body": inner_body
                }}},
                {"id": "bend", "data": {"type": "end", "config": {"outputs": []}}}
            ],
            "edges": [
                {"source": "bstart", "sourceHandle": "out", "target": "it"},
                {"source": "it", "sourceHandle": "out", "target": "bend"}
            ]
        })
    }
    fn leaf_body() -> Value {
        json!({
            "nodes": [
                {"id": "bstart", "data": {"type": "start", "config": {}}},
                {"id": "bend", "data": {"type": "end", "config": {"outputs": []}}}
            ],
            "edges": [{"source": "bstart", "sourceHandle": "out", "target": "bend"}]
        })
    }

    #[test]
    fn iteration_nesting_depth_and_await_rules() {
        // depth 3 (self + 2 nested bodies) OK
        let d3 = iter_body_with(iter_body_with(leaf_body()));
        assert!(
            validate_node(
                T_ITERATION,
                1,
                &json!({"items": {"literal": [1]}, "body": d3})
            )
            .is_ok()
        );

        // depth 4 rejected
        let d4 = iter_body_with(iter_body_with(iter_body_with(leaf_body())));
        let err = validate_node(
            T_ITERATION,
            1,
            &json!({"items": {"literal": [1]}, "body": d4}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("嵌套深度"), "{err}");

        // await inside body rejected at publish
        let with_await = json!({
            "nodes": [
                {"id": "bstart", "data": {"type": "start", "config": {}}},
                {"id": "w", "data": {"type": "await", "config": {"kind": "approval"}}},
                {"id": "bend", "data": {"type": "end", "config": {"outputs": []}}}
            ],
            "edges": [
                {"source": "bstart", "sourceHandle": "out", "target": "w"},
                {"source": "w", "sourceHandle": "out", "target": "bend"}
            ]
        });
        let err = validate_node(
            T_ITERATION,
            1,
            &json!({"items": {"literal": [1]}, "body": with_await}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("await"), "{err}");
    }

    #[test]
    fn iteration_config_validation() {
        let body = json!({
            "nodes": [
                {"id": "start", "data": {"type": "start", "config": {}}},
                {"id": "e", "data": {"type": "end", "config": {"outputs": []}}}
            ],
            "edges": [{"source": "start", "sourceHandle": "out", "target": "e"}]
        });
        let ok = json!({
            "items": {"ref": ["http_1", "json", "data"]},
            "concurrency": 5,
            "on_item_error": "skip",
            "body": body
        });
        assert!(validate_node(T_ITERATION, 1, &ok).is_ok());

        // items must be a ValueExpr
        assert!(validate_node(T_ITERATION, 1, &json!({"items": "x", "body": body})).is_err());
        // bad on_item_error / concurrency / max_items
        for bad in [
            json!({"items": {"ref": ["a"]}, "on_item_error": "ignore", "body": body}),
            json!({"items": {"ref": ["a"]}, "concurrency": 0, "body": body}),
            json!({"items": {"ref": ["a"]}, "concurrency": 21, "body": body}),
            json!({"items": {"ref": ["a"]}, "max_items": 0, "body": body}),
            json!({"items": {"ref": ["a"]}, "max_items": 999999, "body": body}),
        ] {
            assert!(validate_node(T_ITERATION, 1, &bad).is_err(), "{bad}");
        }
        // missing body / zero start / two ends
        assert!(validate_node(T_ITERATION, 1, &json!({"items": {"ref": ["a"]}})).is_err());
        let two_ends = json!({
            "nodes": [
                {"id": "start", "data": {"type": "start", "config": {}}},
                {"id": "e1", "data": {"type": "end", "config": {"outputs": []}}},
                {"id": "e2", "data": {"type": "end", "config": {"outputs": []}}}
            ],
            "edges": []
        });
        assert!(
            validate_node(
                T_ITERATION,
                1,
                &json!({"items": {"ref": ["a"]}, "body": two_ends})
            )
            .is_err()
        );
    }

    #[test]
    fn iteration_video_body_lint_rules() {
        let body_with_video = json!({
            "nodes": [
                {"id": "bstart", "data": {"type": "start", "config": {}}},
                {"id": "v", "data": {"type": "video", "config": {"model": "m", "prompt": "p"}}}
            ],
            "edges": [
                {"source": "bstart", "sourceHandle": "out", "target": "v"}
            ]
        });
        // video 体：end 可省略
        assert!(
            validate_node(
                T_ITERATION,
                1,
                &json!({
                    "items": {"literal": [1]}, "body": body_with_video
                })
            )
            .is_ok()
        );

        // video 带出边 → 拒绝
        let body_out_edge = json!({
            "nodes": [
                {"id": "bstart", "data": {"type": "start", "config": {}}},
                {"id": "v", "data": {"type": "video", "config": {"model": "m", "prompt": "p"}}},
                {"id": "bend", "data": {"type": "end", "config": {"outputs": []}}}
            ],
            "edges": [
                {"source": "bstart", "sourceHandle": "out", "target": "v"},
                {"source": "v", "sourceHandle": "out", "target": "bend"}
            ]
        });
        assert!(
            validate_node(
                T_ITERATION,
                1,
                &json!({
                    "items": {"literal": [1]}, "body": body_out_edge
                })
            )
            .is_err(),
            "video 有出边须拒绝"
        );
    }
}

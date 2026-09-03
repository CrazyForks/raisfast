//! Node registry + config schemas (contracts.md C1).
//!
//! v1 types: start/end/script/egress/branch (+ await config reserved for P2).
//! `validate_node(type, version, config)` deserializes config into a strong Rust
//! struct — shape errors surface as 400 here, not at runtime. Unknown keys are
//! tolerated (extra=allow) so frontend can carry display fields; required keys
//! and value shapes are enforced.

use serde::Deserialize;
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};

/// Reserved handle names (contracts.md C1.3).
pub const H_IN: &str = "in";
pub const H_OUT: &str = "out";
pub const H_ERROR_OUT: &str = "error_out";

/// Known node types for v1.
pub const T_START: &str = "start";
pub const T_END: &str = "end";
pub const T_SCRIPT: &str = "script";
pub const T_EGRESS: &str = "egress";
pub const T_BRANCH: &str = "branch";
pub const T_AWAIT: &str = "await";

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct StartParam {
    pub name: String,
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
    pub name: String,
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
    #[serde(default)]
    pub response_field: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct BranchRule {
    #[serde(default)]
    pub id: String,
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

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Deserialize)]
pub struct AwaitConfig {
    pub kind: String,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub form: Option<Value>,
    #[serde(default)]
    pub approvers: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_secs: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub events: Option<Vec<Value>>,
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
/// or version → `BadRequest`.
pub fn validate_node(kind: &str, _version: i64, config: &Value) -> AppResult<()> {
    let type_error =
        |e: serde_json::Error| AppError::BadRequest(format!("node '{kind}' config invalid: {e}"));
    match kind {
        T_START => {
            let c: StartConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
            for p in &c.params {
                if p.name.is_empty() {
                    return Err(AppError::BadRequest("start.params[].name 不能为空".into()));
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
        T_AWAIT => {
            let _c: AwaitConfig = serde_json::from_value(config.clone()).map_err(type_error)?;
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "node type '{other}' not supported (v1: start|end|script|egress|branch)"
            )));
        }
    }
    Ok(())
}

/// Node type string union (TS literal union; wire = `data.type`).
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
}

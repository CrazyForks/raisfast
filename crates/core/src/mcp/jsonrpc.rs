//! JSON-RPC 2.0 envelope types for the MCP 2026-07-28 transport.
//!
//! Implements the modern (stateless, per-request-metadata) protocol: each
//! request carries `_meta` with `protocolVersion`, `clientInfo`, and
//! `clientCapabilities`; every result includes `resultType`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The MCP protocol version implemented by this server.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// MCP-defined error codes (JSON-RPC reserved range `-32020` to `-32099`).
impl ErrorObject {
    // Standard JSON-RPC
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    // MCP-specific (2026-07-28)
    pub const HEADER_MISMATCH: i32 = -32020;
    pub const MISSING_CLIENT_CAPABILITY: i32 = -32021;
    pub const UNSUPPORTED_PROTOCOL_VERSION: i32 = -32022;

    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn parse_error() -> Self {
        Self::new(Self::PARSE_ERROR, "Parse error")
    }

    pub fn invalid_request() -> Self {
        Self::new(Self::INVALID_REQUEST, "Invalid Request")
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            Self::METHOD_NOT_FOUND,
            format!("Method not found: {method}"),
        )
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self::new(Self::INVALID_PARAMS, msg)
    }

    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self::new(Self::INTERNAL_ERROR, msg)
    }

    /// `-32021` — the request requires a client capability the client didn't declare.
    pub fn missing_client_capability(cap: &str) -> Self {
        Self::new(
            Self::MISSING_CLIENT_CAPABILITY,
            format!("Missing required client capability: {cap}"),
        )
        .with_data(serde_json::json!({ "requiredCapabilities": [cap] }))
    }

    /// `-32022` — the requested protocol version is not supported.
    pub fn unsupported_protocol_version(requested: &str) -> Self {
        Self::new(
            Self::UNSUPPORTED_PROTOCOL_VERSION,
            "Unsupported protocol version",
        )
        .with_data(serde_json::json!({
            "supported": [PROTOCOL_VERSION],
            "requested": requested,
        }))
    }

    /// `-32020` — HTTP header value doesn't match the request body.
    pub fn header_mismatch(detail: impl Into<String>) -> Self {
        Self::new(Self::HEADER_MISMATCH, detail)
    }
}

impl From<ErrorObject> for Value {
    fn from(e: ErrorObject) -> Self {
        serde_json::to_value(&e).unwrap_or_else(|_| {
            Value::String(format!(
                "{{\"code\":{},\"message\":\"serialization failed\"}}",
                e.code
            ))
        })
    }
}

/// An inbound JSON-RPC message: either a request (has `id`) or a notification.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Must be `"2.0"`.
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    /// `None` → notification (no response expected).
    #[serde(default)]
    pub id: Option<Id>,
}

/// JSON-RPC id — a number or a string (null is treated as a notification).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Id {
    Num(i64),
    Str(String),
}

/// Build a successful JSON-RPC response with `resultType: "complete"`.
pub fn success(id: Id, mut result: Value) -> Value {
    // Inject resultType if the caller didn't already.
    if let Some(obj) = result.as_object_mut() {
        obj.entry("resultType")
            .or_insert_with(|| Value::String("complete".into()));
    }
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Build a JSON-RPC error response object for the given id.
pub fn error(id: Id, err: ErrorObject) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": err })
}

/// Best-effort parse of a raw JSON string into a [`Request`].
///
/// Returns the parse error so the caller can reply with `id = null` per spec.
pub fn parse(raw: &str) -> Result<Request, ErrorObject> {
    serde_json::from_str::<Request>(raw).map_err(|e| {
        let code = if raw.trim().is_empty() || raw.trim().parse::<Value>().is_err() {
            ErrorObject::PARSE_ERROR
        } else {
            ErrorObject::INVALID_REQUEST
        };
        ErrorObject::new(code, e.to_string())
    })
}

/// Extract the protocol version from a request's `_meta` field.
///
/// Returns `None` if `_meta` or the version key is absent.
pub fn extract_protocol_version(params: &Value) -> Option<String> {
    params
        .get("_meta")?
        .get("io.modelcontextprotocol/protocolVersion")?
        .as_str()
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_request_with_numeric_id() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let req = parse(raw).expect("valid request should parse");
        assert_eq!(req.method, "tools/list");
        assert!(matches!(req.id, Some(Id::Num(1))));
    }

    #[test]
    fn parse_notification_has_no_id() {
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req = parse(raw).expect("notification should parse");
        assert_eq!(req.method, "notifications/initialized");
        assert!(req.id.is_none());
    }

    #[test]
    fn parse_string_id() {
        let raw = r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#;
        let req = parse(raw).expect("string id should parse");
        assert!(matches!(req.id, Some(Id::Str(_))));
    }

    #[test]
    fn parse_garbage_returns_parse_error() {
        let err = parse("not json at all").unwrap_err();
        assert_eq!(err.code, ErrorObject::PARSE_ERROR);
    }

    #[test]
    fn parse_valid_json_wrong_shape_returns_invalid_request() {
        let err = parse(r#"{"foo":"bar"}"#).unwrap_err();
        assert_eq!(err.code, ErrorObject::INVALID_REQUEST);
    }

    #[test]
    fn success_envelope_includes_result_type() {
        let resp = success(Id::Num(1), serde_json::json!({"tools": []}));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["resultType"], "complete");
        assert!(resp["result"]["tools"].is_array());
    }

    #[test]
    fn error_envelope_shape() {
        let resp = error(Id::Str("x".into()), ErrorObject::method_not_found("boom"));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], "x");
        assert_eq!(resp["error"]["code"], -32601);
        assert!(resp["error"]["message"].as_str().unwrap().contains("boom"));
    }

    #[test]
    fn extract_version_from_meta() {
        let params = serde_json::json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28"
            }
        });
        assert_eq!(extract_protocol_version(&params), Some("2026-07-28".into()));
    }

    #[test]
    fn extract_version_missing_returns_none() {
        let params = serde_json::json!({});
        assert_eq!(extract_protocol_version(&params), None);
    }

    #[test]
    fn unsupported_version_error_carries_supported_list() {
        let err = ErrorObject::unsupported_protocol_version("2099-01-01");
        assert_eq!(err.code, -32022);
        assert_eq!(err.data.unwrap()["supported"][0], PROTOCOL_VERSION);
    }
}

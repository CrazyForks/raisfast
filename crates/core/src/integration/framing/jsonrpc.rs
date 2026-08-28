//! JSON-RPC 2.0 framing (integration.md §2) — thin parse/emit helpers shared
//! by ws connectors (Slack Socket Mode, `eth_subscribe`) and later transports.
//! Wire codec follows MCP's conventions; correlation beyond fire-and-log is
//! added when a connector actually needs response matching.

use serde_json::{Value, json};

/// One parsed JSON-RPC frame.
#[derive(Debug, Clone)]
pub struct JsonRpcFrame {
    /// Present on requests/notifications; absent on responses.
    pub method: Option<String>,
    /// Present on requests/responses; absent on notifications.
    pub id: Option<Value>,
    /// `params` (requests/notifications) or `result` (responses).
    pub payload: Value,
}

/// Parse a text frame. Returns `None` when the payload is not JSON-RPC
/// (no method and no id) — raw-JSON channels rely on this.
#[must_use]
pub fn parse(text: &str) -> Option<JsonRpcFrame> {
    let v: Value = serde_json::from_str(text).ok()?;
    let obj = v.as_object()?;
    if !obj.contains_key("jsonrpc") {
        return None;
    }
    let has_method = obj.contains_key("method");
    let has_id = obj.contains_key("id");
    if !has_method && !has_id {
        return None;
    }
    Some(JsonRpcFrame {
        method: obj.get("method").and_then(Value::as_str).map(str::to_string),
        id: obj.get("id").cloned(),
        payload: obj
            .get("params")
            .or_else(|| obj.get("result"))
            .cloned()
            .unwrap_or(Value::Null),
    })
}

impl JsonRpcFrame {
    /// A notification carries a method but no id (never answered).
    #[must_use]
    pub fn is_notification(&self) -> bool {
        self.method.is_some() && self.id.is_none()
    }

    /// A response to a prior request (id, no method).
    #[must_use]
    pub fn is_response(&self) -> bool {
        self.method.is_none() && self.id.is_some()
    }
}

/// Build a request frame (id expected to be echoed by the peer).
#[must_use]
pub fn request(id: i64, method: &str, params: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0", "id": id, "method": method, "params": params
    }))
    .unwrap_or_default()
}

/// Build a success response frame (ack-by-reply, §5.3).
#[must_use]
pub fn response(id: Value, result: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0", "id": id, "result": result
    }))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_vs_response_vs_raw() {
        let notif = parse(r#"{"jsonrpc":"2.0","method":"events","params":{"a":1}}"#)
            .expect("notification");
        assert!(notif.is_notification());
        assert_eq!(notif.payload["a"], 1);

        let resp = parse(r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#).expect("response");
        assert!(resp.is_response());
        assert_eq!(resp.id, Some(json!(7)));

        assert!(parse(r#"{"id":1,"text":"no jsonrpc marker"}"#).is_none());
        assert!(parse(r#"{"id":1}"#).is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn request_and_response_builders() {
        let req = request(3, "slack.connections.open", json!({}));
        assert!(req.contains("\"method\":\"slack.connections.open\""));
        assert!(req.contains("\"id\":3"));

        let resp = response(json!("ev-9"), json!({}));
        assert!(resp.contains("\"id\":\"ev-9\""));
        assert!(resp.contains("\"result\""));
    }
}

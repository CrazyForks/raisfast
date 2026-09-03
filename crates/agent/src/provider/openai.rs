//! OpenAI-compatible chat provider (OpenAI, DeepSeek, OpenRouter, Ollama `/v1`…).
//!
//! Wire-shape conventions adapted primarily from zeroclaw
//! `crates/zeroclaw-providers/src/openai.rs` (MIT/Apache-2.0), with claw-code
//! `rust/crates/api/src/providers/openai_compat.rs` (MIT) as secondary source;
//! see `dev-docs/agent/reference-analysis.md` C-C1.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{ChatRequest, ChatResponse, ModelProvider, ProviderError};
use crate::messages::{ChatMessage, ChatRole, TokenUsage, ToolCall};
use crate::tool::ToolSpec;

/// `POST {base_url}/chat/completions`.
const ENDPOINT: &str = "chat/completions";

pub struct OpenAiCompatProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl OpenAiCompatProvider {
    /// `base_url` is the API root, e.g. `https://api.openai.com/v1` or
    /// `http://localhost:11434/v1`. `api_key` is optional (Ollama / no-auth).
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client build is infallible");
        Self {
            http,
            base_url: base_url.into(),
            api_key,
        }
    }
}

#[async_trait]
impl ModelProvider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        "openai_compat"
    }

    async fn chat(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), ENDPOINT);

        let mut payload = Map::new();
        payload.insert("model".into(), Value::String(model.to_string()));
        payload.insert(
            "messages".into(),
            Value::Array(request.messages.iter().map(to_wire_message).collect()),
        );
        if let Some(tools) = request.tools
            && !tools.is_empty()
        {
            payload.insert(
                "tools".into(),
                Value::Array(tools.iter().map(to_wire_tool).collect()),
            );
        }
        if let Some(temperature) = request.temperature {
            payload.insert("temperature".into(), Value::from(temperature));
        }

        let mut http_req = self.http.post(&url).json(&Value::Object(payload));
        if let Some(key) = &self.api_key {
            http_req = http_req.bearer_auth(key);
        }

        let resp = http_req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http { status, body: text });
        }

        let parsed: OpenAiChatResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;

        Ok(parsed.into_response())
    }
}

// ── wire mapping ────────────────────────────────────────────────────────────

fn to_wire_role(role: ChatRole) -> &'static str {
    role.as_wire()
}

fn to_wire_message(msg: &ChatMessage) -> Value {
    let mut m = Map::new();
    m.insert("role".into(), Value::String(to_wire_role(msg.role).into()));

    // content may legitimately be null (assistant with only tool_calls).
    m.insert(
        "content".into(),
        msg.content
            .as_deref()
            .map_or(Value::Null, |c| Value::String(c.to_string())),
    );

    // Only attach tool_calls when non-empty: some providers reject an explicit
    // empty array on assistant messages.
    if let Some(calls) = &msg.tool_calls
        && !calls.is_empty()
    {
        let wire: Vec<Value> = calls
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": c.arguments }
                })
            })
            .collect();
        m.insert("tool_calls".into(), Value::Array(wire));
    }

    if let Some(id) = &msg.tool_call_id {
        m.insert("tool_call_id".into(), Value::String(id.clone()));
    }

    Value::Object(m)
}

fn to_wire_tool(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.parameters,
        }
    })
}

// ── response types ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

impl OpenAiChatResponse {
    fn into_response(self) -> ChatResponse {
        // content may be a string or (rarely) a structured value.
        let message = self.choices.into_iter().next().map(|c| c.message);
        let (text, tool_calls) = match message {
            Some(m) => {
                let text = m.content.as_ref().and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Null => None,
                    other => Some(other.to_string()),
                });
                let calls = m
                    .tool_calls
                    .unwrap_or_default()
                    .into_iter()
                    .map(Into::into)
                    .collect();
                (text, calls)
            }
            None => (None, Vec::new()),
        };
        ChatResponse {
            text,
            tool_calls,
            usage: self.usage.map(Into::into),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<Value>,
    // Some providers emit `"tool_calls": null`; Option<Vec> handles both.
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCall {
    #[serde(default)]
    id: Option<String>,
    function: OpenAiFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAiFunction {
    name: String,
    #[serde(default)]
    arguments: String,
}

impl From<OpenAiToolCall> for ToolCall {
    fn from(c: OpenAiToolCall) -> Self {
        let id = c.id.unwrap_or_default();
        // Synthesise an id when the provider omitted one, so history pairing
        // and events stay consistent.
        let id = if id.is_empty() {
            format!("call_{}", c.function.name)
        } else {
            id
        };
        Self {
            id,
            name: c.function.name,
            arguments: c.function.arguments,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

impl From<OpenAiUsage> for TokenUsage {
    fn from(u: OpenAiUsage) -> Self {
        Self {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        }
    }
}

use serde_json::json;

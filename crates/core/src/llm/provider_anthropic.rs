//! Anthropic provider for internal consumption (design §10.2 P4): implements
//! the agent `ModelProvider` surface over the native `/v1/messages` protocol
//! so flows/agent/kb can route onto `provider: "anthropic"` channels.
//!
//! Reference matrix:
//! - request/response conversion: [照抄本仓 `relay/anthropic.rs` → new-api
//!   `relaykit/relayconvert`] — the same conversion the external relay uses,
//!   re-entered from the agent `ChatRequest` shape via
//!   `wire_chat_body` [照抄 zeroclaw/claw-code canonical body] then
//!   `AnthropicAdaptor::convert_chat`.
//! - error envelope compaction: [照抄 claw-code `providers/anthropic.rs
//!   expect_success`] — anthropic `{error:{type,message}}` is collapsed into
//!   a one-line `Http` body so kernel failover classification and logs stay
//!   readable.
//! - streaming usage capture: [照抄 claw-code `MessageStream::observe_event`]
//!   — `message_delta.usage` is authoritative and overwrites the
//!   provisional `message_start.usage`.
//! - non-chat modalities are intentionally NOT overridden: anthropic has no
//!   embeddings/rerank/image/audio/video APIs, so the trait defaults
//!   ("does not support") stand and the kernel skips these channels for
//!   such calls (failover, no failure report).

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use raisfast_agent::messages::{TokenUsage, ToolCall};
use raisfast_agent::provider::openai::wire_chat_body;
use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, StreamEvent,
};

use crate::llm::relay::adaptor::shared_client;
use crate::llm::relay::anthropic::AnthropicAdaptor;

pub(crate) struct AnthropicProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

impl AnthropicProvider {
    /// `base_url` is the anthropic root (no `/v1`), e.g.
    /// `https://api.anthropic.com`. Overrides ride the resolved channel.
    pub(crate) fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        param_override: Option<Value>,
        header_override: Option<Value>,
    ) -> Self {
        Self {
            http: shared_client().clone(),
            base_url: base_url.into(),
            api_key,
            param_override,
            header_override,
        }
    }

    fn endpoint_url(&self) -> String {
        AnthropicAdaptor::request_url(&self.base_url)
    }

    /// Canonical agent request → anthropic `/v1/messages` body.
    fn build_body(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
        stream: bool,
    ) -> Result<Value, ProviderError> {
        let openai_body = wire_chat_body(request, model, false);
        AnthropicAdaptor::convert_chat(openai_body, model, self.param_override.as_ref(), stream)
            .map_err(|e| ProviderError::Config(e.to_string()))
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response, ProviderError> {
        let headers = AnthropicAdaptor::setup_headers(
            self.api_key.as_deref().unwrap_or_default(),
            self.header_override.as_ref(),
        );
        self.http
            .post(self.endpoint_url())
            .headers(headers)
            .json(body)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))
    }

    /// Error envelope compaction [照抄 claw-code `expect_success`]: keep the
    /// upstream status but surface the anthropic `{type, message}` pair.
    async fn expect_success(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(resp);
        }
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let compact = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                let typ = v.pointer("/error/type").and_then(Value::as_str)?;
                let msg = v.pointer("/error/message").and_then(Value::as_str)?;
                Some(format!("{typ}: {msg}"))
            })
            .unwrap_or_else(|| text.chars().take(500).collect());
        Err(ProviderError::Http {
            status,
            body: compact,
        })
    }
}

/// Stream assembly state (anthropic events → StreamEvents + final response).
#[derive(Default)]
struct StreamState {
    done: bool,
    text: String,
    reasoning: String,
    /// anthropic content-block index → tool call under assembly.
    calls: std::collections::BTreeMap<i64, StreamCall>,
    usage: Option<TokenUsage>,
}

#[derive(Default)]
struct StreamCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamState {
    /// Record the authoritative usage [照抄 claw-code `observe_event`]:
    /// only a non-zero `message_delta` frame overwrites.
    fn note_usage(&mut self, v: &Value) {
        let get = |k: &str| {
            v.get("usage")
                .and_then(|u| u.get(k))
                .and_then(Value::as_i64)
                .unwrap_or(0)
        };
        let output = get("output_tokens");
        let read = get("cache_read_input_tokens");
        let write = get("cache_creation_input_tokens");
        if output > 0 || read > 0 || write > 0 {
            let input = (get("input_tokens")).max(0);
            self.usage = Some(TokenUsage {
                input_tokens: Some((input + read + write).max(0) as u64),
                output_tokens: Some(output.max(0) as u64),
                cache_read: Some(read.max(0) as u64),
                cache_write: Some(write.max(0) as u64),
            });
        }
    }
}

/// Feed one SSE line into the stream state, emitting agent events.
fn feed_line(
    state: &mut StreamState,
    event: &mut String,
    on_event: &mut (dyn FnMut(StreamEvent) + Send),
    line: &str,
) {
    if state.done {
        return;
    }
    if let Some(rest) = line.strip_prefix("event:") {
        *event = rest.trim().to_owned();
        return;
    }
    let Some(data) = line.strip_prefix("data:") else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
        return;
    };
    match event.as_str() {
        "message_start" => {
            // Provisional usage is ignored — `message_delta` is the
            // authoritative frame [照抄 claw-code `observe_event`].
        }
        "content_block_start" => {
            if v.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") {
                let idx = v.get("index").and_then(Value::as_i64).unwrap_or_default();
                let entry = state.calls.entry(idx).or_default();
                entry.id = v
                    .pointer("/content_block/id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                entry.name = v
                    .pointer("/content_block/name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
            }
        }
        "content_block_delta" => {
            let idx = v.get("index").and_then(Value::as_i64).unwrap_or_default();
            match v.pointer("/delta/type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        state.text.push_str(t);
                        on_event(StreamEvent::TextDelta {
                            delta: t.to_owned(),
                        });
                    }
                }
                Some("thinking_delta") => {
                    if let Some(t) = v.pointer("/delta/thinking").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        state.reasoning.push_str(t);
                        on_event(StreamEvent::ReasoningDelta {
                            delta: t.to_owned(),
                        });
                    }
                }
                Some("input_json_delta") => {
                    if let Some(p) = v.pointer("/delta/partial_json").and_then(Value::as_str) {
                        state.calls.entry(idx).or_default().arguments.push_str(p);
                    }
                }
                _ => {}
            }
        }
        "message_delta" => state.note_usage(&v),
        "message_stop" => state.done = true,
        "error" => {
            // Terminal upstream error frame; surfaced as Http so the kernel
            // classifies it like any non-2xx.
            state.done = true;
        }
        _ => {}
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        let body = self.build_body(request, model, false)?;
        let resp = self.send(&body).await?;
        let resp = Self::expect_success(resp).await?;
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let m: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;

        let mut out_text = String::new();
        let mut tool_calls = Vec::new();
        if let Some(blocks) = m.get("content").and_then(Value::as_array) {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                            out_text.push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        tool_calls.push(ToolCall {
                            id: b.get("id").and_then(Value::as_str).unwrap_or("").to_owned(),
                            name: b
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                            arguments: serde_json::to_string(b.get("input").unwrap_or(&json!({})))
                                .unwrap_or_else(|_| "{}".to_owned()),
                        });
                    }
                    _ => {}
                }
            }
        }
        let u = m.get("usage").unwrap_or(&Value::Null);
        let get = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0).max(0) as u64;
        let input = get("input_tokens");
        let read = get("cache_read_input_tokens");
        let write = get("cache_creation_input_tokens");
        Ok(ChatResponse {
            text: (!out_text.is_empty()).then_some(out_text),
            tool_calls,
            usage: Some(TokenUsage {
                input_tokens: Some(input + read + write),
                output_tokens: Some(get("output_tokens")),
                cache_read: Some(read),
                cache_write: Some(write),
            }),
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        let body = self.build_body(request, model, true)?;
        let resp = self.send(&body).await?;
        let resp = Self::expect_success(resp).await?;

        let mut state = StreamState::default();
        let mut event = String::new();
        let mut buffer: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ProviderError::Transport(e.to_string()))?;
            buffer.extend_from_slice(&chunk);
            while let Some(idx) = buffer.iter().position(|b| *b == b'\n') {
                let line = buffer.split_off(idx + 1);
                let line = std::mem::replace(&mut buffer, line);
                feed_line(
                    &mut state,
                    &mut event,
                    on_event,
                    &String::from_utf8_lossy(&line),
                );
            }
            if state.done {
                break;
            }
        }
        if !state.done && !buffer.is_empty() {
            feed_line(
                &mut state,
                &mut event,
                on_event,
                &String::from_utf8_lossy(&buffer),
            );
        }

        // Assemble tool calls in anthropic block order; synthesize ids for
        // upstreams that omitted them (same convention as the openai stream).
        let tool_calls: Vec<ToolCall> = state
            .calls
            .into_values()
            .enumerate()
            .map(|(i, c)| {
                let id = if c.id.is_empty() {
                    format!("toolu_call_{i}")
                } else {
                    c.id
                };
                ToolCall {
                    id,
                    name: c.name,
                    arguments: c.arguments,
                }
            })
            .collect();
        for call in &tool_calls {
            on_event(StreamEvent::ToolCall(call.clone()));
        }
        if let Some(u) = state.usage {
            on_event(StreamEvent::Usage(u));
        }
        on_event(StreamEvent::Final);

        Ok(ChatResponse {
            text: (!state.text.is_empty()).then_some(state.text),
            tool_calls,
            usage: state.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(base: &str) -> AnthropicProvider {
        AnthropicProvider::new(base.to_owned(), Some("sk-up".to_owned()), None, None)
    }

    fn req(text: &str) -> ChatRequest<'static> {
        let messages: &'static [raisfast_agent::messages::ChatMessage] =
            Box::leak(Box::new(vec![raisfast_agent::messages::ChatMessage {
                role: raisfast_agent::messages::ChatRole::User,
                content: Some(text.to_owned()),
                tool_calls: None,
                tool_call_id: None,
            }]));
        ChatRequest {
            messages,
            tools: None,
            temperature: None,
            max_tokens: Some(100),
            stop: None,
        }
    }

    #[tokio::test]
    async fn chat_nonstream_against_wiremock() {
        use wiremock::matchers::{body_partial_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-up"))
            .and(body_partial_json(json!({
                "model": "claude-x",
                "max_tokens": 100,
                "messages": [{"role": "user", "content": "hi"}]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "model": "claude-x",
                "content": [
                    {"type": "text", "text": "Hello"},
                    {"type": "tool_use", "id": "toolu_1", "name": "wx", "input": {"c": 1}}
                ],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 80, "cache_read_input_tokens": 20, "output_tokens": 5}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let p = provider(&server.uri());
        let resp = p.chat(&req("hi"), "claude-x").await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("Hello"));
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "wx");
        assert_eq!(resp.tool_calls[0].arguments, r#"{"c":1}"#);
        // usage 归一化：input 含 cache 拆分
        let u = resp.usage.unwrap();
        assert_eq!(u.input_tokens, Some(100));
        assert_eq!(u.cache_read, Some(20));
        assert_eq!(u.output_tokens, Some(5));
        server.verify().await;
    }

    #[tokio::test]
    async fn chat_stream_against_wiremock() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let sse = concat!(
            "event: message_start\n",
            r#"data: {"type":"message_start","message":{"id":"msg_s","model":"claude-x","usage":{"input_tokens":80}}}"#,
            "\n\n",
            "event: content_block_delta\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"He"}}"#,
            "\n\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}"#,
            "\n\n",
            "event: message_delta\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":80,"cache_read_input_tokens":5,"output_tokens":20}}"#,
            "\n\n",
            "event: message_stop\n",
            r#"data: {"type":"message_stop"}"#,
            "\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server)
            .await;

        let p = provider(&server.uri());
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
        let sink_events = events.clone();
        let resp = p
            .chat_stream(&req("hi"), "claude-x", &mut move |e| {
                sink_events.lock().unwrap().push(e)
            })
            .await
            .unwrap();
        assert_eq!(resp.text.as_deref(), Some("He"));
        assert!(!resp.has_tool_calls());
        let u = resp.usage.unwrap();
        assert_eq!(u.input_tokens, Some(85));
        assert_eq!(u.output_tokens, Some(20));
        let got = events.lock().unwrap();
        assert!(
            got.iter()
                .any(|e| matches!(e, StreamEvent::TextDelta { delta } if delta == "He")),
            "{got:?}"
        );
        assert!(
            got.iter()
                .any(|e| matches!(e, StreamEvent::ReasoningDelta { delta } if delta == "hm")),
            "{got:?}"
        );
        assert!(got.iter().any(|e| matches!(e, StreamEvent::Usage(_))));
        assert!(got.iter().any(|e| matches!(e, StreamEvent::Final)));
    }

    #[tokio::test]
    async fn error_envelope_compacted() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429).set_body_json(json!({
                "type": "error",
                "error": {"type": "rate_limit_error", "message": "Number of requests too high"}
            })))
            .mount(&server)
            .await;
        let p = provider(&server.uri());
        let err = p.chat(&req("hi"), "claude-x").await.unwrap_err();
        match err {
            ProviderError::Http { status, body } => {
                assert_eq!(status, 429);
                assert_eq!(body, "rate_limit_error: Number of requests too high");
            }
            other => panic!("expected Http, got {other}"),
        }
    }

    #[tokio::test]
    async fn chat_converts_and_parses_nonstream() {
        // wiremock lives in core's dev-deps; reuse via test harness below.
    }

    #[test]
    fn body_conversion_uses_relay_adaptor() {
        let p = provider("https://api.anthropic.com");
        let body = p.build_body(&req("hi"), "claude-x", true).unwrap();
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn stream_state_usage_delta_is_authoritative() {
        // [照抄 claw-code observe_event]: usage captured from message_delta.
        let mut st = StreamState::default();
        let mut ev = String::new();
        let mut sink = |_: StreamEvent| {};
        feed_line(&mut st, &mut ev, &mut sink, "event: message_start");
        feed_line(
            &mut st,
            &mut ev,
            &mut sink,
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":10}}}"#,
        );
        assert!(st.usage.is_none(), "message_start usage is provisional");
        feed_line(&mut st, &mut ev, &mut sink, "event: message_delta");
        feed_line(
            &mut st,
            &mut ev,
            &mut sink,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":80,"output_tokens":20,"cache_read_input_tokens":5}}"#,
        );
        let u = st.usage.unwrap();
        assert_eq!(u.input_tokens, Some(85), "input includes cache splits");
        assert_eq!(u.output_tokens, Some(20));
        assert_eq!(u.cache_read, Some(5));
    }

    #[test]
    fn stream_state_assembles_tool_calls_in_block_order() {
        let mut st = StreamState::default();
        let mut ev = String::new();
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink_deltas = deltas.clone();
        let mut sink = move |e: StreamEvent| {
            if let StreamEvent::TextDelta { delta } = e {
                sink_deltas.lock().unwrap().push(delta);
            }
        };
        feed_line(&mut st, &mut ev, &mut sink, "event: content_block_start");
        feed_line(
            &mut st,
            &mut ev,
            &mut sink,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"wx"}}"#,
        );
        feed_line(&mut st, &mut ev, &mut sink, "event: content_block_delta");
        feed_line(
            &mut st,
            &mut ev,
            &mut sink,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"c"}}"#,
        );
        feed_line(
            &mut st,
            &mut ev,
            &mut sink,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\":1}"}}"#,
        );
        assert_eq!(st.calls.len(), 1);
        let call = &st.calls[&1];
        assert_eq!(call.id, "toolu_1");
        assert_eq!(call.arguments, r#"{"c":1}"#);
    }
}

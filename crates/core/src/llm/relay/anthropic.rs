//! Anthropic protocol adaptor (design §8.2, P4): the Anthropic-native face
//! of the relay — `/v1/messages` in, `/v1/messages` out.
//!
//! Reference matrix — mappings ported from new-api (vendored at
//! `third/new-api`), file by file:
//! - stop_reason ↔ finish_reason table: [照抄 `relaykit/reasonmap/reasonmap.go`]
//!   (incl. `pause_turn` → `length`, unknown reasons pass through)
//! - openai chat → claude `/v1/messages` request: [照抄
//!   `relaykit/relayconvert/internal/oai_chat/to_claude_messages_req.go`]
//!   (system→text-block array, role normalization, consecutive same-role
//!   string merge, consecutive tool merge into one user message, `...`
//!   placeholders, first-message-must-be-user, max_tokens chain
//!   max_completion_tokens → max_tokens → default, tool_choice + parallel
//!   tool calls [照抄 `shared/claude/tool_choice.go`], input_schema
//!   defaults [照抄 `shared/claude/schema.go`])
//! - claude → openai chat request: [照抄
//!   `relaykit/relayconvert/internal/claude_messages/to_oai_chat_req.go`]
//!   (tool_result→`role:"tool"` with resolved tool `name`, non-string
//!   tool_result content JSON-encoded, tool messages emitted before the
//!   parent user message, single-stop→string, empty-message skip)
//! - non-stream responses: [照抄 `to_claude_messages_resp.go`
//!   `ResponseOpenAI2Claude`] and [照抄 `to_oai_chat_resp.go`
//!   `ResponseClaude2OpenAI`] (thinking→`reasoning_content`, tool
//!   arguments JSON round-trip, response id/model passthrough)
//! - usage normalization both ways: [照抄 `shared/claude/usage.go` +
//!   `to_oai_chat_resp.go buildOpenAIStyleUsageFromClaudeUsage`] — openai
//!   prompt = input + cache_read + cache_write (§9.3); claude input =
//!   prompt − cache splits clamped ≥ 0; `prompt_tokens_details` carries
//!   `cached_tokens` + `cache_write_tokens`
//! - streaming: [照抄 `to_claude_messages_resp.go
//!   `StreamResponseOpenAI2Claude`] (block switch closes+advances the open
//!   block, tool_use blocks start only with id+name, pending args buffer,
//!   message_delta carries the full claude usage) and [照抄
//!   `to_oai_chat_resp.go` + `ClaudeToChatStreamState`] (tool_calls dense
//!   index remap, `signature_delta` → reasoning newline)
//! - default max_tokens: [照抄 `setting/model_setting/claude.go`
//!   `DefaultMaxTokens["default"] = 8192`]
//!
//! [自造] the SSE translation plumbing — our line-buffered frame pipeline
//! (design §8.4) retargeted to event-typed anthropic frames. Documented
//! deltas: `cache_control` blocks are not round-tripped; reasoning effort
//! mapping is dropped (v1 has no openai-side reasoning config either);
//! image URL sources pass through instead of being fetched to base64; the
//! final openai usage frame is always sent (our upstreams are forced to
//! include_usage); passthrough streams forward `message_delta` verbatim
//! without new-api's usage patch.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::Stream;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::llm::relay::adaptor::{RelayUsage, apply_header_override};

/// `anthropic-version` header value [照抄 `relay/channel/claude/adaptor.go`
/// `SetupRequestHeader` default].
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic requires `max_tokens` on every request. Default [照抄
/// `setting/model_setting/claude.go` `DefaultMaxTokens["default"] = 8192`]
/// (new-api's per-model map is admin-configurable; ours is pinned until a
/// channel config knob ships).
pub(crate) const DEFAULT_MAX_TOKENS: i64 = 8192;

fn get_i64(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn now_ts() -> i64 {
    crate::utils::tz::now_utc().timestamp()
}

fn sse_data(v: &Value) -> String {
    format!("data: {v}\n\n")
}

fn update_usage_cell(cell: &Mutex<RelayUsage>, u: RelayUsage) {
    if let Ok(mut c) = cell.lock() {
        *c = u;
    }
}

fn add_chars(cell: &Mutex<usize>, n: usize) {
    if n > 0
        && let Ok(mut c) = cell.lock()
    {
        *c += n;
    }
}

// ── reason maps [照抄 relaykit/reasonmap/reasonmap.go] ───────────

/// OpenAI `finish_reason` ← Anthropic `stop_reason`.
pub(crate) fn finish_reason_of(stop_reason: Option<&str>) -> String {
    match stop_reason {
        Some("stop_sequence") | Some("end_turn") => "stop".to_owned(),
        Some("max_tokens") => "length".to_owned(),
        Some("tool_use") => "tool_calls".to_owned(),
        // Responses has no pause_reason; new-api treats the resumable
        // server-side loop as incomplete rather than a clean stop.
        Some("pause_turn") => "length".to_owned(),
        Some("refusal") => "content_filter".to_owned(),
        Some(other) => other.to_owned(),
        // Absent stop_reason (truncated stream) settles as a normal stop.
        None => "stop".to_owned(),
    }
}

/// Anthropic `stop_reason` ← OpenAI `finish_reason`.
pub(crate) fn stop_reason_of(finish_reason: Option<&str>) -> String {
    match finish_reason {
        Some("stop") => "end_turn".to_owned(),
        Some("stop_sequence") => "stop_sequence".to_owned(),
        Some("length") | Some("max_tokens") => "max_tokens".to_owned(),
        Some("content_filter") => "refusal".to_owned(),
        Some("tool_calls") => "tool_use".to_owned(),
        Some(other) => other.to_owned(),
        // new-api defaults the empty finish to end_turn at the call sites
        None => "end_turn".to_owned(),
    }
}

// ── content helpers ──────────────────────────────────────────────

/// OpenAI `image_url.url` → Anthropic image `source` block: data URIs split
/// into base64 source, http(s) URLs pass through as url source ([自造
/// superset — new-api fetches URLs to base64, which needs async I/O]).
fn image_source(url: &str) -> Option<Value> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let media_type = meta.split(';').next()?;
        if media_type.is_empty() || data.is_empty() {
            return None;
        }
        Some(json!({"type": "base64", "media_type": media_type, "data": data}))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        Some(json!({"type": "url", "url": url}))
    } else {
        None
    }
}

/// Anthropic image `source` block → OpenAI `image_url.url` (data URI or URL).
fn image_url_of(source: &Value) -> Option<String> {
    match source.get("type").and_then(Value::as_str) {
        Some("base64") => {
            let media_type = source
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            let data = source.get("data").and_then(Value::as_str)?;
            Some(format!("data:{media_type};base64,{data}"))
        }
        Some("url") => source.get("url").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    }
}

/// One OpenAI `tool_calls` entry → `(id, name, parsed_input)`.
fn tool_call_parts(tc: &Value, index: usize) -> (String, String, Value) {
    let id = tc
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("toolu_call_{index}"));
    let name = tc
        .pointer("/function/name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = tc
        .pointer("/function/arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let input = match args {
        Value::String(s) => serde_json::from_str(&s).unwrap_or_else(|_| json!({})),
        other => other,
    };
    (id, name, input)
}

/// OpenAI tool parameters → Anthropic `input_schema` [照抄
/// `shared/claude/schema.go FunctionParametersToInputSchema`]: copy keys,
/// default `type: "object"` and `properties: {}`.
fn input_schema_of(parameters: Option<&Value>) -> Value {
    let mut schema = match parameters {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    schema
        .entry("type".to_owned())
        .or_insert_with(|| json!("object"));
    schema
        .entry("properties".to_owned())
        .or_insert_with(|| json!({}));
    Value::Object(schema)
}

/// OpenAI `tool_choice` → Anthropic `tool_choice` [照抄
/// `shared/claude/tool_choice.go MapOpenAIToolChoice`], plus
/// `parallel_tool_calls` → `disable_parallel_tool_use` (never on `none`).
fn tool_choice_to_anthropic(tc: Option<&Value>, parallel: Option<&Value>) -> Option<Value> {
    let mut choice = match tc? {
        Value::String(s) => match s.as_str() {
            "auto" => Some(json!({"type": "auto"})),
            "required" => Some(json!({"type": "any"})),
            "none" => Some(json!({"type": "none"})),
            _ => None,
        },
        Value::Object(_) => {
            let name = tc
                .and_then(|t| t.pointer("/function/name"))
                .and_then(Value::as_str)?;
            Some(json!({"type": "tool", "name": name}))
        }
        _ => None,
    }?;
    if let Some(p) = parallel.and_then(Value::as_bool) {
        let is_none = choice.get("type").and_then(Value::as_str) == Some("none");
        if !is_none && let Some(o) = choice.as_object_mut() {
            o.insert("disable_parallel_tool_use".to_owned(), json!(!p));
        }
    }
    Some(choice)
}

/// Anthropic `tool_choice` → OpenAI `tool_choice` ([自造 superset — new-api
/// drops tool_choice on this direction; ours preserves the client intent]).
fn tool_choice_to_openai(tc: &Value) -> Option<Value> {
    let typ = tc.get("type").and_then(Value::as_str)?;
    match typ {
        "auto" => Some(json!("auto")),
        "any" => Some(json!("required")),
        "none" => Some(json!("none")),
        "tool" => {
            let name = tc.get("name").and_then(Value::as_str)?;
            Some(json!({"type": "function", "function": {"name": name}}))
        }
        _ => None,
    }
}

/// Collect system text from an OpenAI system/developer message.
fn collect_system_text(m: &Value, out: &mut Vec<String>) {
    match m.get("content") {
        Some(Value::String(s)) => out.push(s.clone()),
        Some(Value::Array(parts)) => {
            for p in parts {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    out.push(t.to_owned());
                }
            }
        }
        _ => {}
    }
}

// ── adaptor ──────────────────────────────────────────────────────

/// The Anthropic adaptor: `provider == "anthropic"` channels.
pub(crate) struct AnthropicAdaptor;

impl AnthropicAdaptor {
    /// Upstream URL. Anthropic convention: base has no `/v1` (the SDK appends
    /// `/v1/messages` itself) [照抄 `relay/channel/claude/adaptor.go
    /// GetRequestURL`].
    pub(crate) fn request_url(base_url: &str) -> String {
        format!("{}/v1/messages", base_url.trim_end_matches('/'))
    }

    /// Auth headers [照抄 `adaptor.go SetupRequestHeader`]: `x-api-key` +
    /// `anthropic-version` (client-provided version wins), then overrides.
    pub(crate) fn setup_headers(
        key: &str,
        header_override: Option<&serde_json::Value>,
    ) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::try_from(key) {
            headers.insert("x-api-key", value);
        }
        if let Ok(value) = HeaderValue::try_from(ANTHROPIC_VERSION) {
            headers.insert("anthropic-version", value);
        }
        apply_header_override(&mut headers, header_override);
        headers
    }

    /// OpenAI chat request → Anthropic `/v1/messages` request [照抄
    /// `to_claude_messages_req.go OpenAIChatRequestToClaudeMessages`].
    pub(crate) fn convert_chat(
        body: serde_json::Value,
        upstream_model: &str,
        param_override: Option<&serde_json::Value>,
        stream: bool,
    ) -> AppResult<serde_json::Value> {
        let Some(obj) = body.as_object() else {
            return Err(AppError::BadRequest(
                "request body must be an object".to_owned(),
            ));
        };
        let Some(msgs) = obj.get("messages").and_then(Value::as_array) else {
            return Err(AppError::BadRequest("messages must be an array".to_owned()));
        };

        let mut system_parts: Vec<String> = Vec::new();
        // Accumulated claude messages: (role, content) where content is a
        // JSON string or an array of content blocks.
        let mut out: Vec<(String, Value)> = Vec::new();

        /// Append with new-api's two merge rules: consecutive same-role
        /// string messages join with a space; consecutive `role:"tool"`
        /// messages merge tool_result blocks into one user message.
        fn push_message(out: &mut Vec<(String, Value)>, role: &str, content: Value) {
            if role == "tool" {
                let tool_result = &content[0];
                if let Some((last_role, last_content)) = out.last_mut()
                    && last_role == "user"
                {
                    if last_content.is_string() {
                        let text = last_content.as_str().unwrap_or_default().to_owned();
                        *last_content = json!([{"type": "text", "text": text}]);
                    }
                    if let Some(arr) = last_content.as_array_mut() {
                        arr.push(tool_result.clone());
                        return;
                    }
                }
                out.push(("user".to_owned(), json!([tool_result])));
                return;
            }
            if let (Some((last_role, Value::String(last_text))), Value::String(text)) =
                (out.last_mut(), &content)
                && last_role == role
            {
                last_text.push(' ');
                last_text.push_str(text);
                return;
            }
            out.push((role.to_owned(), content));
        }

        for m in msgs {
            let raw_role = m.get("role").and_then(Value::as_str).unwrap_or("user");
            // Role normalization [照抄 to_claude_messages_req.go:129-147].
            let role = match raw_role {
                "" => "user",
                "developer" => "system",
                "function" => {
                    if m.get("tool_call_id").and_then(Value::as_str).is_some() {
                        "tool"
                    } else {
                        "user"
                    }
                }
                "tool" => {
                    if m.get("tool_call_id").and_then(Value::as_str).is_some() {
                        "tool"
                    } else {
                        "user"
                    }
                }
                "system" | "user" | "assistant" => raw_role,
                _ => "user",
            };
            if role == "system" {
                collect_system_text(m, &mut system_parts);
                continue;
            }
            if role == "tool" {
                // tool_result content [照抄 to_oai_chat_req.go:206-212
                // inverse]: string stays, array is JSON-encoded.
                let content = match m.get("content") {
                    Some(Value::String(s)) => json!(s),
                    Some(other) => other.clone(),
                    None => json!(""),
                };
                push_message(
                    &mut out,
                    "tool",
                    json!([{
                        "type": "tool_result",
                        "tool_use_id": m.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
                        "content": content,
                    }]),
                );
                continue;
            }
            let is_assistant = role == "assistant";
            let tool_calls = m.get("tool_calls").and_then(Value::as_array);
            match m.get("content") {
                Some(Value::String(s)) if !s.is_empty() && tool_calls.is_none() => {
                    push_message(&mut out, role, json!(s));
                }
                content => {
                    let mut blocks: Vec<Value> = Vec::new();
                    if let Some(Value::String(s)) = content
                        && !s.is_empty()
                    {
                        blocks.push(json!({"type": "text", "text": s}));
                    }
                    if let Some(Value::Array(parts)) = content {
                        for p in parts {
                            match p.get("type").and_then(Value::as_str) {
                                Some("text") => {
                                    if let Some(t) = p.get("text").and_then(Value::as_str)
                                        && !t.is_empty()
                                    {
                                        blocks.push(json!({"type": "text", "text": t}));
                                    }
                                }
                                Some("image_url") => {
                                    if let Some(url) =
                                        p.pointer("/image_url/url").and_then(Value::as_str)
                                        && let Some(source) = image_source(url)
                                    {
                                        blocks.push(json!({"type": "image", "source": source}));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if is_assistant && let Some(tcs) = tool_calls {
                        // Empty assistant content with tool calls still
                        // carries a text block [照抄 the "..." placeholder].
                        if blocks.is_empty() {
                            blocks.push(json!({"type": "text", "text": "..."}));
                        }
                        for (i, tc) in tcs.iter().enumerate() {
                            let (id, name, input) = tool_call_parts(tc, i);
                            blocks.push(
                                json!({"type": "tool_use", "id": id, "name": name, "input": input}),
                            );
                        }
                    }
                    if blocks.is_empty() {
                        push_message(&mut out, role, json!("..."));
                    } else {
                        push_message(&mut out, role, Value::Array(blocks));
                    }
                }
            }
        }

        // Anthropic requires the first message to be user [照抄
        // to_claude_messages_req.go placeholderUserMessage].
        if out.is_empty() && !system_parts.is_empty()
            || out.first().is_some_and(|(r, _)| r != "user")
        {
            out.insert(
                0,
                ("user".to_owned(), json!([{"type": "text", "text": "..."}])),
            );
        }

        let messages: Vec<Value> = out
            .into_iter()
            .map(|(role, content)| json!({"role": role, "content": content}))
            .collect();

        let mut root = serde_json::Map::new();
        root.insert("model".to_owned(), json!(upstream_model));
        if !system_parts.is_empty() {
            // System as a text-block array [照抄 to_claude_messages_req.go
            // systemMessages], direct concat without separator.
            root.insert(
                "system".to_owned(),
                Value::Array(
                    system_parts
                        .into_iter()
                        .map(|t| json!({"type": "text", "text": t}))
                        .collect(),
                ),
            );
        }
        root.insert("messages".to_owned(), Value::Array(messages));
        // max_tokens chain [照抄 to_claude_messages_req.go:75-79 +
        // ErrMissingMaxTokens → we always fall back to the 8192 default
        // instead of failing (documented [自造-钉死] delta)].
        let max_tokens = obj
            .get("max_completion_tokens")
            .and_then(Value::as_i64)
            .filter(|v| *v > 0)
            .or_else(|| {
                obj.get("max_tokens")
                    .and_then(Value::as_i64)
                    .filter(|v| *v > 0)
            })
            .unwrap_or(DEFAULT_MAX_TOKENS);
        root.insert("max_tokens".to_owned(), json!(max_tokens));
        if let Some(t) = obj.get("temperature").and_then(Value::as_f64) {
            root.insert("temperature".to_owned(), json!(t));
        }
        if let Some(t) = obj.get("top_p").and_then(Value::as_f64) {
            root.insert("top_p".to_owned(), json!(t));
        }
        if let Some(t) = obj.get("top_k").and_then(Value::as_i64) {
            root.insert("top_k".to_owned(), json!(t));
        }
        match obj.get("stop") {
            Some(Value::String(s)) if !s.is_empty() => {
                root.insert("stop_sequences".to_owned(), json!([s]));
            }
            Some(Value::Array(a)) if !a.is_empty() => {
                let seqs: Vec<Value> = a
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| json!(s))
                    .collect();
                if !seqs.is_empty() {
                    root.insert("stop_sequences".to_owned(), Value::Array(seqs));
                }
            }
            _ => {}
        }
        if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
            let converted: Vec<Value> = tools
                .iter()
                .filter_map(|t| {
                    let f = t.get("function")?;
                    let name = f.get("name").and_then(Value::as_str)?;
                    let mut tool = serde_json::Map::new();
                    tool.insert("name".to_owned(), json!(name));
                    if let Some(d) = f.get("description") {
                        tool.insert("description".to_owned(), d.clone());
                    }
                    tool.insert(
                        "input_schema".to_owned(),
                        input_schema_of(f.get("parameters")),
                    );
                    Some(Value::Object(tool))
                })
                .collect();
            if !converted.is_empty() {
                root.insert("tools".to_owned(), Value::Array(converted));
            }
        }
        let parallel = obj.get("parallel_tool_calls");
        if (obj.get("tool_choice").is_some() || parallel.is_some_and(|p| p.is_boolean()))
            && let Some(v) = tool_choice_to_anthropic(obj.get("tool_choice"), parallel)
        {
            root.insert("tool_choice".to_owned(), v);
        }
        if stream {
            root.insert("stream".to_owned(), json!(true));
        }

        let mut body = Value::Object(root);
        if let Some(Value::Object(over)) = param_override
            && let Some(o) = body.as_object_mut()
        {
            for (k, v) in over {
                o.insert(k.clone(), v.clone());
            }
        }
        Ok(body)
    }

    /// Anthropic-native channel passthrough (design §8.3 inbound mirror):
    /// model rewrite + param_override shallow merge + `stream` flag +
    /// `max_tokens` default. Body stays client-shaped — no lossy round trip.
    pub(crate) fn convert_native(
        mut body: serde_json::Value,
        upstream_model: &str,
        param_override: Option<&serde_json::Value>,
        stream: bool,
    ) -> AppResult<serde_json::Value> {
        let Some(obj) = body.as_object_mut() else {
            return Err(AppError::BadRequest(
                "request body must be an object".to_owned(),
            ));
        };
        obj.insert("model".to_owned(), json!(upstream_model));
        let has_max_tokens = matches!(
            obj.get("max_tokens"),
            Some(Value::Number(n)) if n.as_i64().unwrap_or(0) > 0
        );
        if !has_max_tokens {
            obj.insert("max_tokens".to_owned(), json!(DEFAULT_MAX_TOKENS));
        }
        if stream {
            obj.insert("stream".to_owned(), json!(true));
        }
        if let Some(Value::Object(over)) = param_override {
            for (k, v) in over {
                obj.insert(k.clone(), v.clone());
            }
        }
        Ok(body)
    }

    /// Anthropic `/v1/messages` request → OpenAI canonical chat body [照抄
    /// `claude_messages/to_oai_chat_req.go ClaudeMessagesRequestToOpenAIChat`].
    pub(crate) fn to_openai_body(v: serde_json::Value) -> AppResult<serde_json::Value> {
        let Some(obj) = v.as_object() else {
            return Err(AppError::BadRequest(
                "request body must be an object".to_owned(),
            ));
        };
        let model = obj.get("model").and_then(Value::as_str).unwrap_or("");
        let mut messages: Vec<Value> = Vec::new();
        match obj.get("system") {
            Some(Value::String(s)) if !s.is_empty() => {
                messages.push(json!({"role": "system", "content": s}));
            }
            Some(Value::Array(blocks)) => {
                // Direct concat, no separator [照抄 to_oai_chat_req.go:143-150].
                let text: String = blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect();
                if !text.is_empty() {
                    messages.push(json!({"role": "system", "content": text}));
                }
            }
            _ => {}
        }
        let Some(msgs) = obj.get("messages").and_then(Value::as_array) else {
            return Err(AppError::BadRequest("messages must be an array".to_owned()));
        };
        // tool_use id → name, resolved for tool_result `name` [照抄
        // SearchToolNameByToolCallId].
        let mut tool_names: BTreeMap<String, String> = BTreeMap::new();
        for m in msgs {
            let is_assistant = m.get("role").and_then(Value::as_str) == Some("assistant");
            let mut text_parts: Vec<String> = Vec::new();
            let mut image_parts: Vec<Value> = Vec::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            let mut tool_results: Vec<Value> = Vec::new();
            // String content stays a bare string; block arrays become parts
            // [照抄 to_oai_chat_req.go IsStringContent branching].
            let mut string_content: Option<String> = None;
            match m.get("content") {
                Some(Value::String(s)) => string_content = Some(s.clone()),
                Some(Value::Array(blocks)) => {
                    for b in blocks {
                        match b.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                if let Some(t) = b.get("text").and_then(Value::as_str) {
                                    text_parts.push(t.to_owned());
                                }
                            }
                            Some("image") => {
                                if let Some(source) = b.get("source")
                                    && let Some(url) = image_url_of(source)
                                {
                                    image_parts.push(
                                        json!({"type": "image_url", "image_url": {"url": url}}),
                                    );
                                }
                            }
                            Some("tool_use") => {
                                let id =
                                    b.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
                                let name = b
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_owned();
                                tool_names.insert(id.clone(), name.clone());
                                tool_calls.push(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": serde_json::to_string(
                                            b.get("input").unwrap_or(&json!({})),
                                        )
                                        .unwrap_or_else(|_| "{}".to_owned()),
                                    },
                                }));
                            }
                            Some("tool_result") => {
                                let content = match b.get("content") {
                                    Some(Value::String(s)) => json!(s),
                                    // Non-string content is JSON-encoded
                                    // [照抄 to_oai_chat_req.go:208-212].
                                    Some(other @ Value::Array(_)) => other.clone(),
                                    _ => json!(""),
                                };
                                let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                                let name = b
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                                    .or_else(|| tool_names.get(id).cloned());
                                let mut tm = serde_json::Map::new();
                                tm.insert("role".to_owned(), json!("tool"));
                                if let Some(n) = name {
                                    tm.insert("name".to_owned(), json!(n));
                                }
                                tm.insert("tool_call_id".to_owned(), json!(id));
                                tm.insert(
                                    "content".to_owned(),
                                    match content {
                                        Value::String(s) => json!(s),
                                        other => serde_json::to_string(&other)
                                            .unwrap_or_else(|_| "{}".to_owned())
                                            .into(),
                                    },
                                );
                                tool_results.push(Value::Object(tm));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            // Tool messages are emitted BEFORE the parent message and empty
            // parents are dropped [照抄 to_oai_chat_req.go:156-227].
            messages.extend(tool_results);
            if is_assistant {
                let mut om = serde_json::Map::new();
                om.insert("role".to_owned(), json!("assistant"));
                om.insert(
                    "content".to_owned(),
                    if let Some(s) = string_content {
                        json!(s)
                    } else if text_parts.is_empty() {
                        Value::Null
                    } else {
                        json!(text_parts.join("\n"))
                    },
                );
                if !tool_calls.is_empty() {
                    om.insert("tool_calls".to_owned(), Value::Array(tool_calls));
                }
                messages.push(Value::Object(om));
            } else if let Some(s) = string_content.filter(|_| image_parts.is_empty()) {
                messages.push(json!({"role": "user", "content": s}));
            } else if !text_parts.is_empty() || !image_parts.is_empty() {
                let mut parts: Vec<Value> = text_parts
                    .iter()
                    .map(|t| json!({"type": "text", "text": t}))
                    .collect();
                parts.extend(image_parts);
                messages.push(json!({"role": "user", "content": parts}));
            }
        }

        let mut out = serde_json::Map::new();
        out.insert("model".to_owned(), json!(model));
        out.insert("messages".to_owned(), Value::Array(messages));
        if let Some(mt) = obj.get("max_tokens").and_then(Value::as_i64) {
            out.insert("max_tokens".to_owned(), json!(mt));
        }
        if let Some(t) = obj.get("temperature").and_then(Value::as_f64) {
            out.insert("temperature".to_owned(), json!(t));
        }
        if let Some(t) = obj.get("top_p").and_then(Value::as_f64) {
            out.insert("top_p".to_owned(), json!(t));
        }
        if let Some(t) = obj.get("top_k").and_then(Value::as_i64) {
            out.insert("top_k".to_owned(), json!(t));
        }
        match obj.get("stop_sequences").and_then(Value::as_array) {
            Some(seq) if seq.len() == 1 => {
                out.insert("stop".to_owned(), seq[0].clone());
            }
            Some(seq) if seq.len() > 1 => {
                out.insert("stop".to_owned(), Value::Array(seq.clone()));
            }
            _ => {}
        }
        if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
            let converted: Vec<Value> = tools
                .iter()
                .filter_map(|t| {
                    let name = t.get("name").and_then(Value::as_str)?;
                    let mut f = serde_json::Map::new();
                    f.insert("name".to_owned(), json!(name));
                    if let Some(d) = t.get("description") {
                        f.insert("description".to_owned(), d.clone());
                    }
                    f.insert(
                        "parameters".to_owned(),
                        input_schema_of(t.get("input_schema")),
                    );
                    Some(json!({"type": "function", "function": Value::Object(f)}))
                })
                .collect();
            if !converted.is_empty() {
                out.insert("tools".to_owned(), Value::Array(converted));
            }
        }
        if let Some(tc) = obj.get("tool_choice")
            && let Some(v) = tool_choice_to_openai(tc)
        {
            out.insert("tool_choice".to_owned(), v);
        }
        Ok(Value::Object(out))
    }

    /// Anthropic message response → OpenAI chat.completion [照抄
    /// `to_oai_chat_resp.go ResponseClaude2OpenAI`]. The response's own
    /// model/id pass through (fallback when absent).
    pub(crate) fn completion_to_openai(m: &Value, fallback_model: &str) -> serde_json::Value {
        let mut text = String::new();
        let mut first_thinking = String::new();
        let mut thinking = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        if let Some(blocks) = m.get("content").and_then(Value::as_array) {
            for (i, b) in blocks.iter().enumerate() {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                    }
                    Some("thinking") => {
                        if let Some(t) = b.get("thinking").and_then(Value::as_str) {
                            if i == 0 {
                                first_thinking = t.to_owned();
                            }
                            thinking.push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        let input = b.get("input").cloned().unwrap_or_else(|| json!({}));
                        tool_calls.push(json!({
                            "id": b.get("id").and_then(Value::as_str).unwrap_or(""),
                            "type": "function",
                            "index": i,
                            "function": {
                                "name": b.get("name").and_then(Value::as_str).unwrap_or(""),
                                "arguments": serde_json::to_string(&input)
                                    .unwrap_or_else(|_| "{}".to_owned()),
                            },
                        }));
                    }
                    _ => {}
                }
            }
        }
        let mut message = serde_json::Map::new();
        message.insert("role".to_owned(), json!("assistant"));
        message.insert(
            "content".to_owned(),
            if text.is_empty() {
                Value::Null
            } else {
                json!(text)
            },
        );
        if !thinking.is_empty() {
            message.insert("reasoning_content".to_owned(), json!(thinking));
        }
        if !tool_calls.is_empty() {
            message.insert("tool_calls".to_owned(), Value::Array(tool_calls));
        }
        let finish = finish_reason_of(m.get("stop_reason").and_then(Value::as_str));
        let usage = m.get("usage").unwrap_or(&Value::Null);
        let input = get_i64(usage, "input_tokens");
        let cache_read = get_i64(usage, "cache_read_input_tokens");
        let cache_write = get_i64(usage, "cache_creation_input_tokens");
        let completion = get_i64(usage, "output_tokens");
        let prompt = input + cache_read + cache_write;
        json!({
            "id": m.get("id").and_then(Value::as_str).unwrap_or("chatcmpl-anthropic"),
            "object": "chat.completion",
            "created": now_ts(),
            "model": m.get("model").and_then(Value::as_str).unwrap_or(fallback_model),
            "choices": [{
                "index": 0,
                "message": Value::Object(message),
                // choice-level reasoning_content [照抄 ResponseClaude2OpenAI
                // choice.ReasoningContent, fed from content[0].thinking]
                "reasoning_content": if first_thinking.is_empty() {
                    Value::Null
                } else {
                    json!(first_thinking)
                },
                "finish_reason": finish,
            }],
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": completion,
                "total_tokens": prompt + completion,
                "prompt_tokens_details": {
                    "cached_tokens": cache_read,
                    "cache_write_tokens": cache_write,
                },
            },
        })
    }

    /// OpenAI chat.completion → Anthropic message response [照抄
    /// `to_claude_messages_resp.go ResponseOpenAI2Claude`] for
    /// `/v1/messages` on openai channels.
    pub(crate) fn completion_to_message(openai: &Value, fallback_model: &str) -> serde_json::Value {
        let choice = openai.pointer("/choices/0");
        let msg = choice.and_then(|c| c.get("message"));
        let reasoning = msg
            .and_then(|m| m.get("reasoning_content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let text = msg
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let tool_calls = msg
            .and_then(|m| m.get("tool_calls"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut blocks: Vec<Value> = Vec::new();
        if !reasoning.is_empty() {
            blocks.push(json!({"type": "thinking", "thinking": reasoning}));
        }
        // Text block emitted when non-empty OR when nothing else exists
        // [照抄 to_claude_messages_resp.go:468].
        if !text.is_empty() || (reasoning.is_empty() && tool_calls.is_empty()) {
            blocks.push(json!({"type": "text", "text": text}));
        }
        for tc in &tool_calls {
            let input = match tc.pointer("/function/arguments") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                Some(other) => other.clone(),
                None => json!({}),
            };
            blocks.push(json!({
                "type": "tool_use",
                "id": tc.get("id").and_then(Value::as_str).unwrap_or(""),
                "name": tc.pointer("/function/name").and_then(Value::as_str).unwrap_or(""),
                "input": input,
            }));
        }
        let finish = choice
            .and_then(|c| c.get("finish_reason"))
            .and_then(Value::as_str);
        // Inverse usage mapping [照抄 `shared/claude/usage.go
        // UsageFromOpenAI`]: claude input excludes the cache splits.
        let u = openai.get("usage");
        let prompt = get_i64(u.unwrap_or(&Value::Null), "prompt_tokens");
        let completion = get_i64(u.unwrap_or(&Value::Null), "completion_tokens");
        let details = u.and_then(|x| x.get("prompt_tokens_details"));
        let cache_read = get_i64(details.unwrap_or(&Value::Null), "cached_tokens");
        let cache_write = get_i64(details.unwrap_or(&Value::Null), "cache_write_tokens");
        let mut usage = serde_json::Map::new();
        usage.insert(
            "input_tokens".to_owned(),
            json!((prompt - cache_read - cache_write).max(0)),
        );
        usage.insert("output_tokens".to_owned(), json!(completion));
        if cache_read > 0 {
            usage.insert("cache_read_input_tokens".to_owned(), json!(cache_read));
        }
        if cache_write > 0 {
            usage.insert("cache_creation_input_tokens".to_owned(), json!(cache_write));
        }
        json!({
            // id/model pass through the upstream response [照抄
            // ResponseOpenAI2Claude].
            "id": openai.get("id").and_then(Value::as_str).unwrap_or(""),
            "type": "message",
            "role": "assistant",
            "model": openai.get("model").and_then(Value::as_str).unwrap_or(fallback_model),
            "content": blocks,
            "stop_reason": stop_reason_of(finish),
            "stop_sequence": Value::Null,
            "usage": Value::Object(usage),
        })
    }

    /// Text char count of an anthropic message response (completion side of
    /// the §8.4 estimation fallback for passthrough responses).
    pub(crate) fn message_text_chars(m: &Value) -> usize {
        m.get("content")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .map(|s| s.chars().count())
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Non-stream passthrough read: full body + anthropic usage extraction.
    pub(crate) async fn read_message_response(
        resp: reqwest::Response,
    ) -> AppResult<(serde_json::Value, RelayUsage)> {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read upstream: {e}")))?;
        let body: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
            AppError::Internal(anyhow::anyhow!(
                "upstream non-JSON body: {}",
                &text[..text.len().min(200)]
            ))
        })?;
        if !status.is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "upstream {}: {}",
                status.as_u16(),
                text.chars().take(500).collect::<String>()
            )));
        }
        let usage = RelayUsage::from_anthropic(body.get("usage").unwrap_or(&Value::Null));
        Ok((body, usage))
    }

    /// Non-stream upstream read + openai-shape conversion (chat pipeline).
    pub(crate) async fn handle_response(
        resp: reqwest::Response,
        fallback_model: &str,
    ) -> AppResult<(serde_json::Value, RelayUsage)> {
        let (raw, usage) = Self::read_message_response(resp).await?;
        Ok((Self::completion_to_openai(&raw, fallback_model), usage))
    }

    /// Stream: anthropic upstream events → openai chat chunks (+ usage cell).
    pub(crate) fn handle_stream(
        resp: reqwest::Response,
        usage_out: Arc<Mutex<RelayUsage>>,
        content_chars_out: Arc<Mutex<usize>>,
        fallback_model: &str,
    ) -> Pin<Box<dyn Stream<Item = Result<Vec<u8>, std::io::Error>> + Send>> {
        Box::pin(TranslateStream::new(
            resp,
            AnthropicToOpenai::new(fallback_model, usage_out, content_chars_out),
        ))
    }

    /// Stream: openai upstream chunks → anthropic events (+ usage cell) —
    /// the `/v1/messages` inbound face on openai-compatible channels.
    /// `prompt_est` fills `message_start.usage.input_tokens` (the openai
    /// final-usage frame only arrives at stream end; anthropic SDKs read
    /// the authoritative usage from `message_delta`).
    pub(crate) fn handle_openai_stream(
        resp: reqwest::Response,
        usage_out: Arc<Mutex<RelayUsage>>,
        content_chars_out: Arc<Mutex<usize>>,
        fallback_model: &str,
        prompt_est: i64,
    ) -> Pin<Box<dyn Stream<Item = Result<Vec<u8>, std::io::Error>> + Send>> {
        Box::pin(TranslateStream::new(
            resp,
            OpenaiToAnthropic::new(fallback_model, prompt_est, usage_out, content_chars_out),
        ))
    }

    /// Stream: anthropic upstream events forwarded verbatim while scanning
    /// usage/content chars (the `/v1/messages` face on anthropic channels).
    pub(crate) fn handle_passthrough_stream(
        resp: reqwest::Response,
        usage_out: Arc<Mutex<RelayUsage>>,
        content_chars_out: Arc<Mutex<usize>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Vec<u8>, std::io::Error>> + Send>> {
        Box::pin(TranslateStream::new(
            resp,
            AnthropicPassthrough::new(usage_out, content_chars_out),
        ))
    }
}

// ── SSE translation plumbing [自造] ──────────────────────────────

/// One direction of SSE frame translation. Translators are pure (no I/O):
/// `feed_line` consumes a raw SSE line, translated bytes accumulate in an
/// internal buffer drained by `take_output`.
trait SseTranslator: Send {
    fn feed_line(&mut self, line: &str);
    fn take_output(&mut self) -> Vec<u8>;
    fn is_done(&self) -> bool;
    /// Upstream EOF: emit closing frames when the stream ended without an
    /// explicit terminal event (robustness against truncated upstreams).
    fn on_eof(&mut self);
}

/// Line-buffered stream wrapper driving a translator over the upstream byte
/// stream (the anthropic counterpart of `SseForwardStream`, design §8.4).
struct TranslateStream<T: SseTranslator> {
    upstream: Pin<Box<dyn Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send>>,
    buffer: String,
    translator: T,
}

impl<T: SseTranslator> TranslateStream<T> {
    fn new(resp: reqwest::Response, translator: T) -> Self {
        use futures::StreamExt;
        Self {
            upstream: Box::pin(resp.bytes_stream().map(|r| r.map(|b| b.to_vec()))),
            buffer: String::new(),
            translator,
        }
    }
}

impl<T: SseTranslator + Unpin> Stream for TranslateStream<T> {
    type Item = Result<Vec<u8>, std::io::Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt;
        let this = self.get_mut();
        if this.translator.is_done() {
            return std::task::Poll::Ready(None);
        }
        match this.upstream.poll_next_unpin(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                this.buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = this.buffer.find('\n') {
                    let line: String = this.buffer.drain(..=pos).collect();
                    let trimmed = line.trim_end_matches(['\n', '\r']);
                    this.translator.feed_line(trimmed);
                }
                let out = this.translator.take_output();
                if out.is_empty() {
                    if this.translator.is_done() {
                        return std::task::Poll::Ready(None);
                    }
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                std::task::Poll::Ready(Some(Ok(out)))
            }
            std::task::Poll::Ready(Some(Err(e))) => {
                std::task::Poll::Ready(Some(Err(std::io::Error::other(e))))
            }
            std::task::Poll::Ready(None) => {
                this.translator.on_eof();
                let out = this.translator.take_output();
                if out.is_empty() {
                    std::task::Poll::Ready(None)
                } else {
                    std::task::Poll::Ready(Some(Ok(out)))
                }
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// Per-tool stream state for OpenaiToAnthropic [照抄 `convmeta
/// ClaudeStreamToolCall`]: a tool_use block starts only once id AND name are
/// known; earlier argument deltas buffer until the block opens.
struct OpenaiToolState {
    block_index: i64,
    id: String,
    name: String,
    started: bool,
    pending_args: String,
}

/// openai upstream chunks → anthropic events (inbound `/v1/messages` on
/// openai-compatible channels) [照抄 `StreamResponseOpenAI2Claude`]: block
/// switches close+advance the open block; the terminal `message_delta`
/// carries the full claude-side usage.
struct OpenaiToAnthropic {
    fallback_model: String,
    prompt_est: i64,
    started: bool,
    stream_id: String,
    stream_model: String,
    usage_out: Arc<Mutex<RelayUsage>>,
    chars_out: Arc<Mutex<usize>>,
    /// Currently open block index (LastMessagesType + Index in new-api).
    index: i64,
    last_type: LastType,
    tools: Vec<OpenaiToolState>,
    tool_by_index: BTreeMap<i64, usize>,
    tool_base: i64,
    finish: Option<String>,
    // claude-side usage fields assembled from the openai usage frame.
    input_tokens: i64,
    cache_read: i64,
    cache_write: i64,
    output_tokens: i64,
    has_usage: bool,
    done: bool,
    out: Vec<u8>,
}

#[derive(PartialEq, Clone, Copy)]
enum LastType {
    None,
    Text,
    Thinking,
    Tools,
}

impl OpenaiToAnthropic {
    fn new(
        fallback_model: &str,
        prompt_est: i64,
        usage_out: Arc<Mutex<RelayUsage>>,
        chars_out: Arc<Mutex<usize>>,
    ) -> Self {
        Self {
            fallback_model: fallback_model.to_owned(),
            prompt_est,
            started: false,
            stream_id: String::new(),
            stream_model: String::new(),
            usage_out,
            chars_out,
            index: 0,
            last_type: LastType::None,
            tools: Vec::new(),
            tool_by_index: BTreeMap::new(),
            tool_base: 0,
            finish: None,
            input_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            output_tokens: 0,
            has_usage: false,
            done: false,
            out: Vec::new(),
        }
    }

    fn emit(&mut self, event: &str, body: Value) {
        self.out
            .extend_from_slice(format!("event: {event}\ndata: {body}\n\n").as_bytes());
    }

    fn push_usage_cell(&self) {
        update_usage_cell(
            &self.usage_out,
            RelayUsage {
                prompt_tokens: self.input_tokens + self.cache_read + self.cache_write,
                completion_tokens: self.output_tokens,
                cache_read_tokens: self.cache_read,
                cache_write_tokens: self.cache_write,
            },
        );
    }

    fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let id = if self.stream_id.is_empty() {
            format!("msg_{}", crate::utils::id::new_id())
        } else {
            std::mem::take(&mut self.stream_id)
        };
        let model = if self.stream_model.is_empty() {
            std::mem::take(&mut self.fallback_model)
        } else {
            std::mem::take(&mut self.stream_model)
        };
        self.emit(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": { "input_tokens": self.prompt_est, "output_tokens": 0 },
                },
            }),
        );
    }

    /// Emit content_block_stop for the currently open block(s) [照抄
    /// stopOpenBlocks].
    fn stop_open_blocks(&mut self) {
        match self.last_type {
            LastType::None => {}
            LastType::Text | LastType::Thinking => {
                let idx = self.index;
                self.emit(
                    "content_block_stop",
                    json!({"type": "content_block_stop", "index": idx}),
                );
            }
            LastType::Tools => {
                let indexes: Vec<i64> = self.tools.iter().map(|t| t.block_index).collect();
                for idx in indexes {
                    self.emit(
                        "content_block_stop",
                        json!({"type": "content_block_stop", "index": idx}),
                    );
                }
            }
        }
    }

    /// Close the open block(s) and advance the index [照抄
    /// stopOpenBlocksAndAdvance] — prevents mismatched block types when the
    /// stream interleaves text/thinking/tools.
    fn stop_and_advance(&mut self) {
        if self.last_type == LastType::None {
            return;
        }
        self.stop_open_blocks();
        if self.last_type == LastType::Tools {
            self.index = self.tool_base + self.tools.len() as i64;
            self.tools.clear();
            self.tool_by_index.clear();
            self.tool_base = 0;
        } else {
            self.index += 1;
        }
        self.last_type = LastType::None;
    }

    /// Terminal sequence: pending tool blocks, block stops, message_delta
    /// with the full claude usage, message_stop [照抄
    /// FinalizeStreamResponseOpenAI2Claude].
    fn finalize_terminal(&mut self) {
        if self.done || !self.started {
            return;
        }
        self.start_pending_tools();
        self.stop_open_blocks();
        let stop_reason = stop_reason_of(self.finish.as_deref());
        let mut usage = serde_json::Map::new();
        usage.insert("input_tokens".to_owned(), json!(self.input_tokens));
        usage.insert("output_tokens".to_owned(), json!(self.output_tokens));
        if self.cache_read > 0 {
            usage.insert("cache_read_input_tokens".to_owned(), json!(self.cache_read));
        }
        if self.cache_write > 0 {
            usage.insert(
                "cache_creation_input_tokens".to_owned(),
                json!(self.cache_write),
            );
        }
        self.emit(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": Value::Null },
                "usage": Value::Object(usage),
            }),
        );
        self.emit("message_stop", json!({"type": "message_stop"}));
        self.done = true;
    }

    /// Open tool blocks that have a name but were still waiting for id/args
    /// when the stream ends [照抄 startPendingToolBlocks].
    fn start_pending_tools(&mut self) {
        if self.last_type != LastType::Tools {
            return;
        }
        let pending: Vec<(i64, String, String, String)> = self
            .tools
            .iter()
            .filter(|t| !t.started && !t.name.is_empty())
            .map(|t| {
                (
                    t.block_index,
                    t.id.clone(),
                    t.name.clone(),
                    t.pending_args.clone(),
                )
            })
            .collect();
        for (idx, id, name, args) in pending {
            let id = if id.is_empty() {
                format!("toolu_{}", crate::utils::id::new_id())
            } else {
                id
            };
            self.emit(
                "content_block_start",
                json!({"type": "content_block_start", "index": idx,
                       "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
            );
            if let Some(t) = self.tools.iter_mut().find(|t| t.block_index == idx) {
                t.started = true;
                t.id = id;
                t.pending_args.clear();
            }
            if !args.is_empty() {
                self.emit(
                    "content_block_delta",
                    json!({"type": "content_block_delta", "index": idx,
                           "delta": {"type": "input_json_delta", "partial_json": args}}),
                );
            }
        }
    }

    fn feed_frame(&mut self, v: &Value) {
        // usage-only frames close a pending terminal (finish-first upstreams)
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let parsed = RelayUsage::from_openai(u);
            if parsed.prompt_tokens > 0 {
                // claude-side input = openai prompt − cache splits
                self.input_tokens =
                    (parsed.prompt_tokens - parsed.cache_read_tokens - parsed.cache_write_tokens)
                        .max(0);
                self.cache_read = parsed.cache_read_tokens;
                self.cache_write = parsed.cache_write_tokens;
            }
            if parsed.completion_tokens > 0 {
                self.output_tokens = parsed.completion_tokens;
            }
            if parsed.prompt_tokens > 0 || parsed.completion_tokens > 0 {
                self.has_usage = true;
                self.push_usage_cell();
            }
        }
        let Some(choice) = v.pointer("/choices/0") else {
            if self.has_usage {
                self.finalize_terminal();
            }
            return;
        };
        if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(f.to_owned());
        }
        if let Some(Value::String(id)) = v.get("id") {
            self.stream_id = id.clone();
        }
        if let Some(Value::String(model)) = v.get("model") {
            self.stream_model = model.clone();
        }
        self.ensure_started();
        if let Some(tcs) = choice
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
        {
            if self.last_type != LastType::Tools {
                self.stop_and_advance();
                self.tool_base = self.index;
                self.tools.clear();
                self.tool_by_index.clear();
            }
            self.last_type = LastType::Tools;
            for tc in tcs {
                let oidx = get_i64(tc, "index");
                let slot = match self.tool_by_index.get(&oidx) {
                    Some(i) => *i,
                    None => {
                        let i = self.tools.len();
                        self.tools.push(OpenaiToolState {
                            block_index: self.tool_base + i as i64,
                            id: String::new(),
                            name: String::new(),
                            started: false,
                            pending_args: String::new(),
                        });
                        self.tool_by_index.insert(oidx, i);
                        i
                    }
                };
                let incoming_id = tc
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let name = tc
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let args = tc
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                {
                    let tool = &mut self.tools[slot];
                    if !incoming_id.is_empty() && tool.id.is_empty() {
                        tool.id = incoming_id;
                    }
                    if !name.is_empty() && tool.name.is_empty() {
                        tool.name = name;
                    }
                }
                let idx = self.tools[slot].block_index;
                let (tool_id, tool_name, started) = {
                    let t = &self.tools[slot];
                    (t.id.clone(), t.name.clone(), t.started)
                };
                if !started {
                    self.tools[slot].pending_args.push_str(&args);
                    if !tool_id.is_empty() && !tool_name.is_empty() {
                        self.emit(
                            "content_block_start",
                            json!({"type": "content_block_start", "index": idx,
                                   "content_block": {"type": "tool_use", "id": tool_id, "name": tool_name, "input": {}}}),
                        );
                        self.tools[slot].started = true;
                        let pending = std::mem::take(&mut self.tools[slot].pending_args);
                        if !pending.is_empty() {
                            self.emit(
                                "content_block_delta",
                                json!({"type": "content_block_delta", "index": idx,
                                       "delta": {"type": "input_json_delta", "partial_json": pending}}),
                            );
                        }
                    }
                } else if !args.is_empty() {
                    self.emit(
                        "content_block_delta",
                        json!({"type": "content_block_delta", "index": idx,
                               "delta": {"type": "input_json_delta", "partial_json": args}}),
                    );
                }
            }
            if !self.tools.is_empty() {
                self.index = self.tool_base + self.tools.len() as i64 - 1;
            }
        } else {
            let reasoning = choice
                .pointer("/delta/reasoning_content")
                .and_then(Value::as_str)
                .unwrap_or("");
            let text = choice
                .pointer("/delta/content")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !reasoning.is_empty() {
                if self.last_type != LastType::Thinking {
                    self.stop_and_advance();
                    let idx = self.index;
                    self.emit(
                        "content_block_start",
                        json!({"type": "content_block_start", "index": idx,
                               "content_block": {"type": "thinking", "thinking": ""}}),
                    );
                }
                self.last_type = LastType::Thinking;
                let idx = self.index;
                self.emit(
                    "content_block_delta",
                    json!({"type": "content_block_delta", "index": idx,
                           "delta": {"type": "thinking_delta", "thinking": reasoning}}),
                );
            } else if !text.is_empty() {
                if self.last_type != LastType::Text {
                    self.stop_and_advance();
                    let idx = self.index;
                    self.emit(
                        "content_block_start",
                        json!({"type": "content_block_start", "index": idx,
                               "content_block": {"type": "text", "text": ""}}),
                    );
                }
                self.last_type = LastType::Text;
                let idx = self.index;
                add_chars(&self.chars_out, text.chars().count());
                self.emit(
                    "content_block_delta",
                    json!({"type": "content_block_delta", "index": idx,
                           "delta": {"type": "text_delta", "text": text}}),
                );
            }
        }
        if self.finish.is_some() && self.has_usage {
            self.finalize_terminal();
        }
    }
}

impl SseTranslator for OpenaiToAnthropic {
    fn feed_line(&mut self, line: &str) {
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim();
        if data == "[DONE]" {
            self.finalize_terminal();
            return;
        }
        if let Ok(v) = serde_json::from_str::<Value>(data) {
            self.feed_frame(&v);
        }
    }

    fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn on_eof(&mut self) {
        self.finalize_terminal();
    }
}

/// anthropic upstream events → openai chat chunks (outbound stream) [照抄
/// `StreamResponseClaude2OpenAI` + `ClaudeToChatStreamState`]: tool_calls
/// carry a dense index (anthropic block indexes remapped so text/thinking
/// blocks leave no holes).
struct AnthropicToOpenai {
    fallback_model: String,
    stream_model: String,
    msg_id: String,
    usage_out: Arc<Mutex<RelayUsage>>,
    chars_out: Arc<Mutex<usize>>,
    event: String,
    /// anthropic content-block index → dense openai tool_calls index.
    tool_index_by_block: BTreeMap<i64, i64>,
    next_tool_index: i64,
    input_tokens: i64,
    cache_read: i64,
    cache_write: i64,
    output_tokens: i64,
    stop_reason: Option<String>,
    saw_start: bool,
    done: bool,
    out: Vec<u8>,
}

impl AnthropicToOpenai {
    fn new(
        fallback_model: &str,
        usage_out: Arc<Mutex<RelayUsage>>,
        chars_out: Arc<Mutex<usize>>,
    ) -> Self {
        Self {
            fallback_model: fallback_model.to_owned(),
            stream_model: String::new(),
            msg_id: String::new(),
            usage_out,
            chars_out,
            event: String::new(),
            tool_index_by_block: BTreeMap::new(),
            next_tool_index: 0,
            input_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            output_tokens: 0,
            stop_reason: None,
            saw_start: false,
            done: false,
            out: Vec::new(),
        }
    }

    fn push_usage(&self) {
        update_usage_cell(
            &self.usage_out,
            RelayUsage {
                prompt_tokens: self.input_tokens + self.cache_read + self.cache_write,
                completion_tokens: self.output_tokens,
                cache_read_tokens: self.cache_read,
                cache_write_tokens: self.cache_write,
            },
        );
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> String {
        sse_data(&json!({
            "id": self.msg_id,
            "object": "chat.completion.chunk",
            "created": now_ts(),
            "model": if self.stream_model.is_empty() { &self.fallback_model } else { &self.stream_model },
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish.map(|f| json!(f)).unwrap_or(Value::Null),
            }],
        }))
    }

    /// include_usage-style trailing frame with the authoritative usage
    /// [照抄 buildOpenAIStyleUsageFromClaudeUsage].
    fn usage_frame(&self) -> String {
        let prompt = self.input_tokens + self.cache_read + self.cache_write;
        sse_data(&json!({
            "id": self.msg_id,
            "object": "chat.completion.chunk",
            "created": now_ts(),
            "model": if self.stream_model.is_empty() { &self.fallback_model } else { &self.stream_model },
            "choices": [],
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": self.output_tokens,
                "total_tokens": prompt + self.output_tokens,
                "prompt_tokens_details": {
                    "cached_tokens": self.cache_read,
                    "cache_write_tokens": self.cache_write,
                },
            },
        }))
    }

    fn close(&mut self) {
        let finish = finish_reason_of(self.stop_reason.as_deref());
        self.out
            .extend_from_slice(self.chunk(json!({}), Some(finish.as_str())).as_bytes());
        self.out.extend_from_slice(self.usage_frame().as_bytes());
        self.out.extend_from_slice(b"data: [DONE]\n\n");
        self.done = true;
    }

    fn feed_frame(&mut self, event: &str, v: &Value) {
        match event {
            "message_start" => {
                if let Some(id) = v.pointer("/message/id").and_then(Value::as_str) {
                    self.msg_id = id.to_owned();
                }
                if let Some(model) = v.pointer("/message/model").and_then(Value::as_str) {
                    self.stream_model = model.to_owned();
                }
                let u = v.pointer("/message/usage").unwrap_or(&Value::Null);
                self.input_tokens = get_i64(u, "input_tokens");
                self.cache_read = get_i64(u, "cache_read_input_tokens");
                self.cache_write = get_i64(u, "cache_creation_input_tokens");
                self.saw_start = true;
                self.push_usage();
                self.out.extend_from_slice(
                    self.chunk(json!({"role": "assistant", "content": ""}), None)
                        .as_bytes(),
                );
            }
            "content_block_start" => {
                let idx = get_i64(v, "index");
                match v.pointer("/content_block/type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = v
                            .pointer("/content_block/text")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if !text.is_empty() {
                            add_chars(&self.chars_out, text.chars().count());
                        }
                        self.out.extend_from_slice(
                            self.chunk(json!({"content": text}), None).as_bytes(),
                        );
                    }
                    Some("tool_use") => {
                        let tool_idx = match self.tool_index_by_block.get(&idx) {
                            Some(i) => *i,
                            None => {
                                let i = self.next_tool_index;
                                self.next_tool_index += 1;
                                self.tool_index_by_block.insert(idx, i);
                                i
                            }
                        };
                        let delta = json!({"tool_calls": [{
                            "index": tool_idx,
                            "id": v.pointer("/content_block/id").and_then(Value::as_str).unwrap_or(""),
                            "type": "function",
                            "function": {
                                "name": v.pointer("/content_block/name").and_then(Value::as_str).unwrap_or(""),
                                "arguments": "",
                            },
                        }]});
                        self.out
                            .extend_from_slice(self.chunk(delta, None).as_bytes());
                    }
                    // thinking / server_tool_use / hosted blocks: no chunk
                    _ => {}
                }
            }
            "content_block_delta" => {
                let idx = get_i64(v, "index");
                match v.pointer("/delta/type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str) {
                            add_chars(&self.chars_out, t.chars().count());
                            self.out.extend_from_slice(
                                self.chunk(json!({"content": t}), None).as_bytes(),
                            );
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(t) = v.pointer("/delta/thinking").and_then(Value::as_str) {
                            self.out.extend_from_slice(
                                self.chunk(json!({"reasoning_content": t}), None).as_bytes(),
                            );
                        }
                    }
                    // [照抄 to_oai_chat_resp.go:87-89] signatures become a
                    // newline in reasoning_content.
                    Some("signature_delta") => {
                        self.out.extend_from_slice(
                            self.chunk(json!({"reasoning_content": "\n"}), None)
                                .as_bytes(),
                        );
                    }
                    Some("input_json_delta") => {
                        if let Some(p) = v.pointer("/delta/partial_json").and_then(Value::as_str) {
                            let tool_idx = match self.tool_index_by_block.get(&idx) {
                                Some(i) => *i,
                                None => {
                                    let i = self.next_tool_index;
                                    self.next_tool_index += 1;
                                    self.tool_index_by_block.insert(idx, i);
                                    i
                                }
                            };
                            let delta = json!({"tool_calls": [{"index": tool_idx, "function": {"arguments": p}}]});
                            self.out
                                .extend_from_slice(self.chunk(delta, None).as_bytes());
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(sr) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(sr.to_owned());
                }
                if let Some(o) = v
                    .get("usage")
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(Value::as_i64)
                {
                    self.output_tokens = o;
                    self.push_usage();
                }
            }
            "message_stop" => self.close(),
            "error" => {
                let typ = v
                    .pointer("/error/type")
                    .and_then(Value::as_str)
                    .unwrap_or("api_error");
                let msg = v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("upstream stream error");
                self.out.extend_from_slice(
                    sse_data(&json!({"error": {"message": msg, "type": typ, "code": null}}))
                        .as_bytes(),
                );
                self.out.extend_from_slice(b"data: [DONE]\n\n");
                self.done = true;
            }
            _ => {}
        }
    }
}

impl SseTranslator for AnthropicToOpenai {
    fn feed_line(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("event:") {
            self.event = rest.trim().to_owned();
            return;
        }
        if let Some(data) = line.strip_prefix("data:")
            && let Ok(v) = serde_json::from_str::<Value>(data.trim())
        {
            // Keep the event type until the next `event:` line — tolerant of
            // both one-event-per-frame and batched data lines.
            let event = self.event.clone();
            self.feed_frame(&event, &v);
        }
    }

    fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn on_eof(&mut self) {
        // Truncated upstream (no message_stop): still emit the openai
        // terminators so clients see a well-formed stream and settle runs.
        if self.saw_start && !self.done {
            self.close();
        }
    }
}

/// anthropic upstream events forwarded verbatim while scanning usage and
/// content chars (inbound `/v1/messages` passthrough on anthropic channels).
struct AnthropicPassthrough {
    usage_out: Arc<Mutex<RelayUsage>>,
    chars_out: Arc<Mutex<usize>>,
    event: String,
    input_tokens: i64,
    cache_read: i64,
    cache_write: i64,
    output_tokens: i64,
    out: Vec<u8>,
}

impl AnthropicPassthrough {
    fn new(usage_out: Arc<Mutex<RelayUsage>>, chars_out: Arc<Mutex<usize>>) -> Self {
        Self {
            usage_out,
            chars_out,
            event: String::new(),
            input_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            output_tokens: 0,
            out: Vec::new(),
        }
    }

    fn push_usage(&self) {
        update_usage_cell(
            &self.usage_out,
            RelayUsage {
                prompt_tokens: self.input_tokens + self.cache_read + self.cache_write,
                completion_tokens: self.output_tokens,
                cache_read_tokens: self.cache_read,
                cache_write_tokens: self.cache_write,
            },
        );
    }
}

impl SseTranslator for AnthropicPassthrough {
    fn feed_line(&mut self, line: &str) {
        // Forward verbatim (newline-normalized — SSE tolerates LF).
        self.out.extend_from_slice(line.as_bytes());
        self.out.push(b'\n');
        if let Some(rest) = line.strip_prefix("event:") {
            self.event = rest.trim().to_owned();
            return;
        }
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            return;
        };
        match self.event.as_str() {
            "message_start" => {
                let u = v.pointer("/message/usage").unwrap_or(&Value::Null);
                self.input_tokens = get_i64(u, "input_tokens");
                self.cache_read = get_i64(u, "cache_read_input_tokens");
                self.cache_write = get_i64(u, "cache_creation_input_tokens");
                self.push_usage();
            }
            "content_block_delta" => {
                if v.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta")
                    && let Some(t) = v.pointer("/delta/text").and_then(Value::as_str)
                {
                    add_chars(&self.chars_out, t.chars().count());
                }
            }
            "message_delta" => {
                if let Some(o) = v
                    .get("usage")
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(Value::as_i64)
                {
                    self.output_tokens = o;
                    self.push_usage();
                }
            }
            _ => {}
        }
    }

    fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    fn is_done(&self) -> bool {
        false
    }

    fn on_eof(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells() -> (Arc<Mutex<RelayUsage>>, Arc<Mutex<usize>>) {
        (
            Arc::new(Mutex::new(RelayUsage::default())),
            Arc::new(Mutex::new(0)),
        )
    }

    // ── reason maps ─────────────────────────────────────────────

    #[test]
    fn stop_reason_table_matches_reasonmap() {
        // [照抄 reasonmap.go] incl. pause_turn → length.
        assert_eq!(finish_reason_of(Some("end_turn")), "stop");
        assert_eq!(finish_reason_of(Some("stop_sequence")), "stop");
        assert_eq!(finish_reason_of(Some("max_tokens")), "length");
        assert_eq!(finish_reason_of(Some("tool_use")), "tool_calls");
        assert_eq!(finish_reason_of(Some("pause_turn")), "length");
        assert_eq!(finish_reason_of(Some("refusal")), "content_filter");

        assert_eq!(stop_reason_of(Some("stop")), "end_turn");
        assert_eq!(stop_reason_of(Some("stop_sequence")), "stop_sequence");
        assert_eq!(stop_reason_of(Some("length")), "max_tokens");
        assert_eq!(stop_reason_of(Some("content_filter")), "refusal");
        assert_eq!(stop_reason_of(Some("tool_calls")), "tool_use");
    }

    // ── openai → claude request ─────────────────────────────────

    #[test]
    fn convert_chat_extracts_system_and_defaults_max_tokens() {
        let body = json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": "be nice"},
                {"role": "developer", "content": "be terse"},
                {"role": "user", "content": "hi"}
            ]
        });
        let out = AnthropicAdaptor::convert_chat(body, "claude-x", None, false).unwrap();
        assert_eq!(out["model"], "claude-x");
        // system as text-block array, direct concat
        assert_eq!(out["system"][0]["type"], "text");
        assert_eq!(out["system"][0]["text"], "be nice");
        assert_eq!(out["system"][1]["text"], "be terse");
        // max_tokens default 8192 [照抄 claude.go DefaultMaxTokens]
        assert_eq!(out["max_tokens"], 8192);
        assert!(out.get("stream").is_none());
    }

    #[test]
    fn convert_chat_max_tokens_chain_and_merge() {
        let body = json!({
            "model": "m",
            "max_completion_tokens": 500,
            "max_tokens": 900,
            "messages": [{"role": "user", "content": "hi"}]
        });
        let over = json!({"max_tokens": 123});
        let out = AnthropicAdaptor::convert_chat(body, "m", Some(&over), false).unwrap();
        // max_completion_tokens wins inside the chain; admin override wins last
        assert_eq!(out["max_tokens"], 123);
        let body2 = json!({
            "model": "m",
            "max_completion_tokens": 500,
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out2 = AnthropicAdaptor::convert_chat(body2, "m", None, false).unwrap();
        assert_eq!(out2["max_tokens"], 500);
    }

    #[test]
    fn convert_chat_merges_same_role_and_tools() {
        let body = json!({
            "model": "m",
            "tools": [{"type": "function", "function": {
                "name": "get_weather", "description": "w",
                "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}}],
            "tool_choice": "required",
            "parallel_tool_calls": false,
            "messages": [
                {"role": "user", "content": "a"},
                {"role": "user", "content": "b"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_1", "type": "function",
                     "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\"}"}}]},
                {"role": "tool", "tool_call_id": "call_1", "content": "sunny"}
            ]
        });
        let out = AnthropicAdaptor::convert_chat(body, "m", None, false).unwrap();
        // consecutive user strings merged with a space
        assert_eq!(out["messages"][0]["content"], "a b");
        // assistant: "..." text block + tool_use with parsed input
        assert_eq!(out["messages"][1]["content"][0]["text"], "...");
        assert_eq!(out["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(out["messages"][1]["content"][1]["input"]["city"], "SF");
        // tool → tool_result merged into the same user message
        assert_eq!(out["messages"][2]["role"], "user");
        assert_eq!(out["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(out["messages"][2]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(out["messages"][2]["content"][0]["content"], "sunny");
        // tools + tool_choice mapping
        assert_eq!(out["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(out["tool_choice"]["type"], "any");
        assert_eq!(out["tool_choice"]["disable_parallel_tool_use"], true);
    }

    #[test]
    fn convert_chat_first_message_must_be_user() {
        let body = json!({
            "model": "m",
            "messages": [{"role": "assistant", "content": "prefill"}]
        });
        let out = AnthropicAdaptor::convert_chat(body, "m", None, false).unwrap();
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][0]["content"][0]["text"], "...");
        assert_eq!(out["messages"][1]["role"], "assistant");
    }

    #[test]
    fn convert_chat_images_and_stop() {
        let body = json!({
            "model": "m",
            "stop": ["END"],
            "top_k": 5,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "look"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
            ]}]
        });
        let out = AnthropicAdaptor::convert_chat(body, "m", None, false).unwrap();
        assert_eq!(out["stop_sequences"], json!(["END"]));
        assert_eq!(out["top_k"], 5);
        assert_eq!(out["messages"][0]["content"][1]["type"], "image");
        assert_eq!(out["messages"][0]["content"][1]["source"]["type"], "base64");
        assert_eq!(
            out["messages"][0]["content"][1]["source"]["media_type"],
            "image/png"
        );
    }

    // ── claude → openai request ─────────────────────────────────

    #[test]
    fn to_openai_body_maps_system_tools_and_stop() {
        let body = json!({
            "model": "claude-3",
            "system": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}],
            "max_tokens": 300,
            "stop_sequences": ["only"],
            "tools": [{"name": "t", "description": "d", "input_schema": {"type": "object"}}],
            "tool_choice": {"type": "any"},
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = AnthropicAdaptor::to_openai_body(body).unwrap();
        assert_eq!(out["model"], "claude-3");
        // single-block system → message; multi-block direct concat
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][0]["content"], "ab");
        // single stop sequence → bare string
        assert_eq!(out["stop"], "only");
        assert_eq!(out["tools"][0]["type"], "function");
        assert_eq!(out["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(out["tool_choice"], "required");
    }

    #[test]
    fn to_openai_body_tool_use_and_result() {
        let body = json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "checking"},
                    {"type": "tool_use", "id": "toolu_1", "name": "wx", "input": {"city": "SF"}}]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": [
                        {"type": "text", "text": "sunny"}]}]}
            ]
        });
        let out = AnthropicAdaptor::to_openai_body(body).unwrap();
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["content"], "weather?");
        assert_eq!(msgs[1]["role"], "assistant", "assistant parent");
        assert_eq!(msgs[1]["content"], "checking");
        assert_eq!(msgs[1]["tool_calls"][0]["id"], "toolu_1");
        assert_eq!(
            msgs[1]["tool_calls"][0]["function"]["arguments"],
            r#"{"city":"SF"}"#
        );
        // tool message emitted AFTER the assistant it answers
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "toolu_1");
        assert_eq!(
            msgs[2]["name"], "wx",
            "tool name resolved from prior tool_use"
        );
        // non-string tool_result content is JSON-encoded
        let content = msgs[2]["content"].as_str().unwrap();
        assert!(content.contains("sunny"), "content: {content}");
    }

    #[test]
    fn to_openai_body_skips_empty_and_maps_images() {
        let body = json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": [{"type": "image", "source": {
                    "type": "base64", "media_type": "image/jpeg", "data": "ZZZ"}}]},
                {"role": "user", "content": []}
            ]
        });
        let out = AnthropicAdaptor::to_openai_body(body).unwrap();
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1, "empty user message dropped");
        assert_eq!(msgs[0]["content"][0]["type"], "image_url");
        assert_eq!(
            msgs[0]["content"][0]["image_url"]["url"],
            "data:image/jpeg;base64,ZZZ"
        );
    }

    // ── responses ───────────────────────────────────────────────

    #[test]
    fn completion_to_openai_maps_blocks_and_usage() {
        let m = json!({
            "id": "msg_1",
            "model": "claude-x",
            "content": [
                {"type": "text", "text": "Hello"},
                {"type": "tool_use", "id": "toolu_9", "name": "t", "input": {"k": 1}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 100, "output_tokens": 40,
                      "cache_read_input_tokens": 60, "cache_creation_input_tokens": 10}
        });
        let out = AnthropicAdaptor::completion_to_openai(&m, "fallback");
        assert_eq!(out["model"], "claude-x", "upstream model passthrough");
        assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(out["choices"][0]["message"]["content"], "Hello");
        assert_eq!(
            out["choices"][0]["message"]["tool_calls"][0]["id"],
            "toolu_9"
        );
        // usage normalization: prompt = input + read + write
        assert_eq!(out["usage"]["prompt_tokens"], 170);
        assert_eq!(out["usage"]["prompt_tokens_details"]["cached_tokens"], 60);
        assert_eq!(
            out["usage"]["prompt_tokens_details"]["cache_write_tokens"],
            10
        );
    }

    #[test]
    fn completion_to_message_inverse_usage() {
        let openai = json!({
            "id": "cmpl-1",
            "model": "up-m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"},
                         "finish_reason": "length"}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 40,
                      "prompt_tokens_details": {"cached_tokens": 60}}
        });
        let out = AnthropicAdaptor::completion_to_message(&openai, "public");
        assert_eq!(out["type"], "message");
        assert_eq!(out["id"], "cmpl-1", "id passthrough [照抄]");
        assert_eq!(out["model"], "up-m");
        assert_eq!(out["stop_reason"], "max_tokens");
        assert_eq!(out["content"][0]["type"], "text");
        // claude input excludes the cache splits
        assert_eq!(out["usage"]["input_tokens"], 40);
        assert_eq!(out["usage"]["output_tokens"], 40);
        assert_eq!(out["usage"]["cache_read_input_tokens"], 60);
    }

    #[test]
    fn completion_to_message_empty_content_gets_text_block() {
        let openai = json!({
            "id": "cmpl-2", "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": null},
                         "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 0}
        });
        let out = AnthropicAdaptor::completion_to_message(&openai, "m");
        assert_eq!(
            out["content"][0]["type"], "text",
            "content never empty [照抄]"
        );
    }

    // ── streaming: anthropic upstream → openai client ───────────

    #[test]
    fn stream_anthropic_to_openai_translates_frames() {
        let (usage, chars) = cells();
        let mut t = AnthropicToOpenai::new("gpt-4o", usage.clone(), chars.clone());
        for line in [
            "event: message_start",
            r#"data: {"type":"message_start","message":{"id":"msg_1","model":"claude-up","usage":{"input_tokens":80,"cache_read_input_tokens":20}}}"#,
            "",
            "event: content_block_start",
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"wx"}}"#,
            "event: content_block_delta",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"c"}}"#,
            "event: message_delta",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":20}}"#,
            "event: message_stop",
            r#"data: {"type":"message_stop"}"#,
        ] {
            t.feed_line(line);
        }
        let out = String::from_utf8(t.take_output()).unwrap();
        assert!(
            out.contains(r#""delta":{"role":"assistant","content":""}"#),
            "{out}"
        );
        assert!(
            out.contains(r#""model":"claude-up""#),
            "upstream model echoed"
        );
        // dense tool index: block 1 → tool index 0 (no hole from text block 0)
        assert!(
            out.contains(r#""tool_calls":[{"index":0,"id":"toolu_1""#),
            "{out}"
        );
        assert!(out.contains(r#""finish_reason":"tool_calls""#));
        assert!(out.contains(r#""cached_tokens":20"#));
        assert!(out.contains("[DONE]"));
        let u = usage.lock().unwrap();
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.cache_read_tokens),
            (100, 20, 20)
        );
        assert_eq!(*chars.lock().unwrap(), 3);
        assert!(t.is_done());
    }

    #[test]
    fn stream_anthropic_eof_without_stop_still_closes() {
        let (usage, chars) = cells();
        let mut t = AnthropicToOpenai::new("m", usage, chars);
        t.feed_line("event: message_start");
        t.feed_line(
            r#"data: {"type":"message_start","message":{"id":"x","usage":{"input_tokens":10}}}"#,
        );
        t.feed_line(r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"par"}}"#);
        assert!(!t.is_done());
        t.on_eof();
        let out = String::from_utf8(t.take_output()).unwrap();
        assert!(
            out.contains(r#""finish_reason":"stop""#),
            "eof closes the stream"
        );
        assert!(out.contains("[DONE]"));
    }

    // ── streaming: openai upstream → anthropic client ───────────

    #[test]
    fn stream_openai_to_anthropic_translates_chunks() {
        let (usage, chars) = cells();
        let mut t = OpenaiToAnthropic::new("gpt-4o", 25, usage.clone(), chars.clone());
        for line in [
            r#"data: {"id":"s1","object":"chat.completion.chunk","model":"up-m","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"data: {"id":"s1","choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
            r#"data: {"id":"s1","choices":[{"index":0,"delta":{"content":"lo"}}]}"#,
            r#"data: {"id":"s1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":80,"completion_tokens":20}}"#,
            "data: [DONE]",
        ] {
            t.feed_line(line);
        }
        let out = String::from_utf8(t.take_output()).unwrap();
        assert!(out.contains("event: message_start"));
        assert!(out.contains(r#""id":"s1""#), "chunk id passthrough [照抄]");
        assert!(out.contains(r#""model":"up-m""#));
        assert!(
            out.contains(r#""input_tokens":25"#),
            "estimate in message_start"
        );
        assert!(out.contains(r#"event: content_block_start"#));
        assert!(out.contains(r#""type":"text_delta","text":"Hel""#));
        assert!(out.contains(r#"event: content_block_stop"#));
        assert!(out.contains(r#""stop_reason":"end_turn""#));
        // message_delta carries the full claude usage [照抄]
        assert!(out.contains(r#""input_tokens":80"#));
        assert!(out.contains(r#""output_tokens":20"#));
        assert!(out.contains("event: message_stop"));
        let u = usage.lock().unwrap();
        assert_eq!((u.prompt_tokens, u.completion_tokens), (80, 20));
        assert_eq!(*chars.lock().unwrap(), 5);
        assert!(t.is_done());
    }

    #[test]
    fn stream_openai_block_switch_closes_and_advances() {
        let (usage, chars) = cells();
        let mut t = OpenaiToAnthropic::new("m", 10, usage, chars);
        for line in [
            r#"data: {"id":"s","choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#,
            // reasoning first (block 0), then text (block 1)
            r#"data: {"id":"s","choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}"#,
            r#"data: {"id":"s","choices":[{"index":0,"delta":{"content":"text"}}]}"#,
            "data: [DONE]",
        ] {
            t.feed_line(line);
        }
        let out = String::from_utf8(t.take_output()).unwrap();
        // thinking block opened at 0, closed when text switched to block 1
        assert!(out.contains(r#""content_block":{"type":"thinking""#));
        assert!(out.contains(r#"event: content_block_stop"#));
        assert!(out.contains(r#""content_block":{"type":"text""#));
        let stops: Vec<&str> = out.matches("event: content_block_stop").collect();
        assert_eq!(stops.len(), 2, "both blocks closed: {out}");
        assert!(out.contains(r#""type":"thinking_delta","thinking":"think""#));
    }

    #[test]
    fn stream_openai_tool_args_defer_until_started() {
        let (usage, chars) = cells();
        let mut t = OpenaiToAnthropic::new("m", 10, usage, chars);
        for line in [
            r#"data: {"id":"s","choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
            // first delta: id+name arrive with args in the same frame
            r#"data: {"id":"s","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"wx","arguments":"{\"c"}}]}}]}"#,
            r#"data: {"id":"s","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]}}]}"#,
            "data: [DONE]",
        ] {
            t.feed_line(line);
        }
        let out = String::from_utf8(t.take_output()).unwrap();
        assert!(out.contains(r#""content_block":{"type":"tool_use","id":"call_1","name":"wx""#));
        // pending args flushed right after the start, then live args
        assert!(out.contains(r#""partial_json":"{\"c""#));
        assert!(out.contains(r#""partial_json":":1}""#));
        assert!(out.contains(r#""stop_reason":"end_turn""#));
    }

    #[test]
    fn stream_openai_finish_before_usage_waits() {
        let (usage, chars) = cells();
        let mut t = OpenaiToAnthropic::new("m", 10, usage.clone(), chars);
        t.feed_line(r#"data: {"id":"s","choices":[{"index":0,"delta":{"role":"assistant"}}]}"#);
        t.feed_line(r#"data: {"id":"s","choices":[{"index":0,"delta":{"content":"x"}}]}"#);
        t.feed_line(
            r#"data: {"id":"s","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        );
        assert!(!t.is_done(), "finish without usage keeps the stream open");
        // usage-only trailing chunk closes it
        t.feed_line(
            r#"data: {"id":"s","choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#,
        );
        assert!(t.is_done());
        let out = String::from_utf8(t.take_output()).unwrap();
        assert!(out.contains(r#""output_tokens":2"#));
        let u = usage.lock().unwrap();
        assert_eq!(u.prompt_tokens, 5);
    }

    // ── passthrough scan ────────────────────────────────────────

    #[test]
    fn stream_passthrough_forwards_and_scans_usage() {
        let (usage, chars) = cells();
        let mut t = AnthropicPassthrough::new(usage.clone(), chars.clone());
        for line in [
            "event: message_start",
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":50}}}"#,
            "event: content_block_delta",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hey"}}"#,
            "event: message_delta",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}"#,
        ] {
            t.feed_line(line);
        }
        let out = String::from_utf8(t.take_output()).unwrap();
        for line in [
            "event: message_start",
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":50}}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}"#,
        ] {
            assert!(out.contains(line), "forwarded verbatim: {line}");
        }
        let u = usage.lock().unwrap();
        assert_eq!((u.prompt_tokens, u.completion_tokens), (50, 9));
        assert_eq!(*chars.lock().unwrap(), 3);
    }

    // ── misc ────────────────────────────────────────────────────

    #[test]
    fn request_url_and_headers() {
        assert_eq!(
            AnthropicAdaptor::request_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            AnthropicAdaptor::request_url("https://x.com/"),
            "https://x.com/v1/messages"
        );
        let h = AnthropicAdaptor::setup_headers("sk-up", None);
        assert_eq!(h.get("x-api-key").unwrap(), "sk-up");
        assert_eq!(h.get("anthropic-version").unwrap(), "2023-06-01");
        let over = json!({"anthropic-beta": "prompt-caching"});
        let h2 = AnthropicAdaptor::setup_headers("k", Some(&over));
        assert_eq!(h2.get("anthropic-beta").unwrap(), "prompt-caching");
    }

    #[test]
    fn convert_native_passthrough_shape() {
        let body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let out = AnthropicAdaptor::convert_native(
            body,
            "claude-up",
            Some(&json!({"temperature": 0.5})),
            true,
        )
        .unwrap();
        assert_eq!(out["model"], "claude-up");
        assert_eq!(out["max_tokens"], 8192);
        assert_eq!(out["stream"], true);
        assert_eq!(out["temperature"], 0.5);
        assert_eq!(out["messages"][0]["role"], "user");
    }
}

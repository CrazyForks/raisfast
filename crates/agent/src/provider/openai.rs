//! OpenAI-compatible chat provider (OpenAI, DeepSeek, OpenRouter, Ollama `/v1`…).
//!
//! Wire-shape conventions adapted primarily from zeroclaw
//! `crates/zeroclaw-providers/src/openai.rs` (MIT/Apache-2.0), with claw-code
//! `rust/crates/api/src/providers/openai_compat.rs` (MIT) as secondary source;
//! see `dev-docs/agent/reference-analysis.md` C-C1.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{
    AudioInput, ChatRequest, ChatResponse, GeneratedImage, ImageRequest, ModelProvider,
    ProviderError, RerankResult, StreamEvent, Transcription, VideoRequest, VideoStatus, VideoTask,
};
use crate::messages::{ChatMessage, ChatRole, TokenUsage, ToolCall};
use crate::tool::ToolSpec;

/// `POST {base_url}/chat/completions`.
const ENDPOINT: &str = "chat/completions";

/// `POST {base_url}/embeddings` (OpenAI-compatible embeddings wire protocol).
const EMBEDDINGS_ENDPOINT: &str = "embeddings";

/// `POST {base_url}/rerank` (Jina/Cohere-compatible reranking).
const RERANK_ENDPOINT: &str = "rerank";

/// `POST {base_url}/images/generations`.
const IMAGES_ENDPOINT: &str = "images/generations";

/// `POST {base_url}/audio/transcriptions` (multipart).
const TRANSCRIPTIONS_ENDPOINT: &str = "audio/transcriptions";

/// `POST {base_url}/audio/speech` (binary audio out).
const SPEECH_ENDPOINT: &str = "audio/speech";

/// `POST {base_url}/videos` (+ `/{id}` and `/{id}/content`).
const VIDEOS_ENDPOINT: &str = "videos";

/// Wire shape of the `/embeddings` response (`data[i].embedding` + `index`).
#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
    #[serde(default)]
    index: usize,
}

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
        let body = self.build_body(request, model, false)?;
        let text = self.send_json(&body).await?;

        let parsed: OpenAiChatResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;

        Ok(parsed.into_response())
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        let body = self.build_body(request, model, true)?;
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), ENDPOINT);

        let mut http_req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            http_req = http_req.bearer_auth(key);
        }
        let resp = http_req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp
                .text()
                .await
                .map_err(|e| ProviderError::Transport(e.to_string()))?;
            return Err(ProviderError::Http { status, body: text });
        }

        let mut state = StreamState::default();
        let mut buffer: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ProviderError::Transport(e.to_string()))?;
            buffer.extend_from_slice(&chunk);
            // Drain complete lines, keep the trailing partial line buffered.
            while let Some(idx) = buffer.iter().position(|b| *b == b'\n') {
                let line = buffer.split_off(idx + 1);
                let line = std::mem::replace(&mut buffer, line);
                feed_line(&mut state, on_event, &String::from_utf8_lossy(&line));
            }
            if state.done {
                break;
            }
        }
        if !state.done && !buffer.is_empty() {
            feed_line(&mut state, on_event, &String::from_utf8_lossy(&buffer));
        }

        // Emit fully assembled tool calls, then finish.
        let tool_calls: Vec<ToolCall> = state
            .calls
            .into_values()
            .map(|c| {
                let id = if c.id.is_empty() {
                    format!("call_{}", c.name)
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
        on_event(StreamEvent::Final);

        Ok(ChatResponse {
            text: (!state.text.is_empty()).then_some(state.text),
            tool_calls,
            usage: state.usage.map(TokenUsage::from),
        })
    }
    async fn embed(&self, texts: &[&str], model: &str) -> Result<Vec<Vec<f32>>, ProviderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::json!({ "model": model, "input": texts });
        let text = self.send_json_to(EMBEDDINGS_ENDPOINT, &body).await?;
        let parsed: OpenAiEmbeddingResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        let mut data = parsed.data;
        if data.len() != texts.len() {
            return Err(ProviderError::Parse(format!(
                "embedding count mismatch: sent {} texts, got {} vectors",
                texts.len(),
                data.len()
            )));
        }
        // Servers may return entries out of order; honor the `index` field.
        data.sort_by_key(|d| d.index);
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }

    async fn rerank(
        &self,
        query: &str,
        documents: &[&str],
        model: &str,
    ) -> Result<Vec<RerankResult>, ProviderError> {
        let body = serde_json::json!({
            "model": model,
            "query": query,
            "documents": documents,
        });
        let text = self.send_json_to(RERANK_ENDPOINT, &body).await?;
        let parsed: OpenAiRerankResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        let mut results: Vec<RerankResult> = parsed
            .results
            .into_iter()
            .map(|r| RerankResult {
                index: r.index,
                relevance_score: r.relevance_score,
            })
            .collect();
        // Providers return best-first; sort defensively so the contract holds
        // even for upstreams that don't.
        results.sort_by(|a, b| b.relevance_score.total_cmp(&a.relevance_score));
        Ok(results)
    }

    async fn generate_image(
        &self,
        request: &ImageRequest,
        model: &str,
    ) -> Result<Vec<GeneratedImage>, ProviderError> {
        let mut body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
            "n": request.n.max(1),
        });
        if let Some(size) = &request.size {
            body["size"] = Value::String(size.clone());
        }
        let text = self.send_json_to(IMAGES_ENDPOINT, &body).await?;
        let parsed: OpenAiImagesResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        Ok(parsed
            .data
            .into_iter()
            .map(|d| GeneratedImage {
                b64_json: d.b64_json,
                url: d.url,
            })
            .collect())
    }

    async fn transcribe(
        &self,
        audio: &AudioInput<'_>,
        model: &str,
    ) -> Result<Transcription, ProviderError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            TRANSCRIPTIONS_ENDPOINT
        );
        let part = reqwest::multipart::Part::bytes(audio.data.to_vec())
            .file_name(audio.filename.clone())
            .mime_str(audio.mime.as_deref().unwrap_or("application/octet-stream"))
            .map_err(|e| ProviderError::Config(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", model.to_owned())
            .part("file", part);
        if let Some(language) = &audio.language {
            form = form.text("language", language.clone());
        }
        let mut http_req = self.http.post(&url).multipart(form);
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
        let parsed: OpenAiTranscriptionResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        Ok(Transcription { text: parsed.text })
    }

    async fn speech(&self, text: &str, voice: &str, model: &str) -> Result<Vec<u8>, ProviderError> {
        let body = serde_json::json!({
            "model": model,
            "input": text,
            "voice": voice,
        });
        self.send_bytes_to(SPEECH_ENDPOINT, &body).await
    }

    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let mut body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
        });
        if let Some(seconds) = &request.seconds {
            body["seconds"] = Value::String(seconds.clone());
        }
        if let Some(size) = &request.size {
            body["size"] = Value::String(size.clone());
        }
        let text = self.send_json_to(VIDEOS_ENDPOINT, &body).await?;
        let parsed: OpenAiVideoTask = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        Ok(parsed.into_task())
    }

    async fn video_query(&self, task_id: &str, _model: &str) -> Result<VideoTask, ProviderError> {
        let text = self
            .send_get_to(&format!("{VIDEOS_ENDPOINT}/{task_id}"))
            .await?;
        let parsed: OpenAiVideoTask = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        Ok(parsed.into_task())
    }

    async fn video_content(&self, task_id: &str, _model: &str) -> Result<Vec<u8>, ProviderError> {
        let url = format!(
            "{}/{}/{}/content",
            self.base_url.trim_end_matches('/'),
            VIDEOS_ENDPOINT,
            task_id
        );
        let mut http_req = self.http.get(&url);
        if let Some(key) = &self.api_key {
            http_req = http_req.bearer_auth(key);
        }
        let resp = http_req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        Ok(bytes.to_vec())
    }
}

impl OpenAiCompatProvider {
    fn build_body(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
        stream: bool,
    ) -> Result<Value, ProviderError> {
        Ok(wire_chat_body(request, model, stream))
    }

    async fn send_json(&self, body: &Value) -> Result<String, ProviderError> {
        self.send_json_to(ENDPOINT, body).await
    }

    /// POST a JSON body to `{base_url}/{path}` with optional bearer auth.
    async fn send_json_to(&self, path: &str, body: &Value) -> Result<String, ProviderError> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), path);
        let mut http_req = self.http.post(&url).json(body);
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
        Ok(text)
    }

    /// POST a JSON body expecting raw bytes back (TTS audio).
    async fn send_bytes_to(&self, path: &str, body: &Value) -> Result<Vec<u8>, ProviderError> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), path);
        let mut http_req = self.http.post(&url).json(body);
        if let Some(key) = &self.api_key {
            http_req = http_req.bearer_auth(key);
        }
        let resp = http_req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        Ok(bytes.to_vec())
    }

    /// GET `{base_url}/{path}` expecting JSON (video task polling).
    async fn send_get_to(&self, path: &str) -> Result<String, ProviderError> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), path);
        let mut http_req = self.http.get(&url);
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
        Ok(text)
    }
}

// ── modality wire response types ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenAiRerankResponse {
    #[serde(default)]
    results: Vec<OpenAiRerankResult>,
}

#[derive(Debug, Deserialize)]
struct OpenAiRerankResult {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    relevance_score: f32,
}

#[derive(Debug, Deserialize)]
struct OpenAiImagesResponse {
    #[serde(default)]
    data: Vec<OpenAiImageData>,
}

#[derive(Debug, Deserialize)]
struct OpenAiImageData {
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiTranscriptionResponse {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiVideoTask {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    progress: Option<i32>,
    #[serde(default)]
    error: Option<Value>,
}

impl OpenAiVideoTask {
    fn into_task(self) -> VideoTask {
        // Wire error is `{code, message}` [照抄 async-openai
        // `VideoResourceError`] — surface the human message.
        let error = match self.error {
            Some(Value::String(s)) => Some(s),
            Some(Value::Object(o)) => o.get("message").and_then(Value::as_str).map(str::to_owned),
            Some(other) => Some(other.to_string()),
            None => None,
        };
        VideoTask {
            id: self.id,
            status: VideoStatus::from_wire(&self.status),
            progress: self.progress,
            error,
        }
    }
}

/// Streaming accumulation state + SSE feed (wire conventions adapted from
/// zeroclaw `zeroclaw-providers/src/openai.rs`; reference-analysis C-C1).
#[derive(Default)]
struct StreamState {
    done: bool,
    text: String,
    reasoning: String,
    calls: BTreeMap<u32, StreamCall>,
    next_index: u32,
    usage: Option<OpenAiUsage>,
}

#[derive(Default)]
struct StreamCall {
    id: String,
    name: String,
    arguments: String,
}

fn feed_line(state: &mut StreamState, on_event: &mut (dyn FnMut(StreamEvent) + Send), line: &str) {
    if state.done {
        return;
    }
    let line = line.trim_end();
    if line.is_empty() {
        return;
    }
    let Some(payload) = line.strip_prefix("data:") else {
        return; // ignore `event:`/`id:`/`: keepalive` lines
    };
    let payload = payload.trim_start();
    if payload == "[DONE]" {
        state.done = true;
        return;
    }
    let chunk: ChatStreamChunk = match serde_json::from_str(payload) {
        Ok(c) => c,
        Err(_) => return,
    };

    if let Some(u) = chunk.usage {
        on_event(StreamEvent::Usage(u.clone().into()));
        state.usage = Some(u);
    }

    for choice in chunk.choices {
        let delta = choice.delta;
        if let Some(content) = delta.content
            && !content.is_empty()
        {
            state.text.push_str(&content);
            on_event(StreamEvent::TextDelta { delta: content });
        }
        // DeepSeek/GLM style reasoning surfaces in streaming deltas.
        for reasoning in [delta.reasoning_content, delta.reasoning]
            .into_iter()
            .flatten()
            .filter(|r| !r.is_empty())
        {
            state.reasoning.push_str(&reasoning);
            on_event(StreamEvent::ReasoningDelta { delta: reasoning });
        }
        if let Some(calls) = delta.tool_calls {
            for call in calls {
                let index = call.index.unwrap_or_else(|| {
                    let i = state.next_index;
                    state.next_index += 1;
                    i
                });
                let entry = state.calls.entry(index).or_default();
                if let Some(id) = call.id {
                    entry.id = id;
                }
                if let Some(function) = call.function {
                    if let Some(name) = function.name {
                        entry.name = name;
                    }
                    if let Some(args) = function.arguments {
                        entry.arguments.push_str(&args);
                    }
                }
            }
        }
    }
}

// ── wire mapping ────────────────────────────────────────────────────────────

/// Serialize a chat request into the OpenAI chat-completions wire body.
/// Public so the host can re-target the canonical body at other protocols
/// (e.g. the anthropic `/v1/messages` adaptor consumes this).
pub fn wire_chat_body(request: &ChatRequest<'_>, model: &str, stream: bool) -> Value {
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
    if let Some(max_tokens) = request.max_tokens {
        payload.insert("max_tokens".into(), Value::from(max_tokens));
    }
    if let Some(stop) = &request.stop
        && !stop.is_empty()
    {
        payload.insert(
            "stop".into(),
            Value::Array(stop.iter().map(|s| Value::String(s.clone())).collect()),
        );
    }
    if stream {
        payload.insert("stream".into(), Value::Bool(true));
        payload.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    Value::Object(payload)
}

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

#[derive(Debug, Clone, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    /// DeepSeek field for auto KV-cache hits.
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u64>,
    /// OpenAI field (non-streaming chat completions): `prompt_tokens_details.cached_tokens`.
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Clone, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

impl From<OpenAiUsage> for TokenUsage {
    fn from(u: OpenAiUsage) -> Self {
        let cache_read = u.prompt_cache_hit_tokens.or_else(|| {
            u.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
        });
        Self {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            cache_read,
            cache_write: None,
        }
    }
}

// ── streaming response types ────────────────────────────────────────────────

/// One SSE `data:` frame of a streaming chat completion.
#[derive(Debug, Deserialize)]
struct ChatStreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Debug, Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallDelta {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct StreamFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

use serde_json::json;

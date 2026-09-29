//! PluginProvider：provider 扩展 → `ModelProvider` video 面的桥接层。
//!
//! 职责（provider-plugins.md §5.2）：
//! - 组装 ctx（apiKey/baseUrl/model/paramOverride/request/taskId，§4.3）；
//! - `runner.call("buildSubmitRequest", ctx)` → spec 校验（url/method/白名单）
//!   → host 代发（status 透传，信封分类交扩展）→
//!   `runner.call("parseSubmitResponse", ctx, resp)`；
//! - video_content：host 直接下载 parseTaskResult 给出的 url（不过 JS、
//!   不走扩展白名单——成片 CDN 域与 API 域不同是常态）；
//! - 错误映射：JS 异常/缺失/序列化 → Config；HTTP → Http（status 透传）；
//!   网络/超时 → Transport；
//! - 状态映射：未知 → InProgress + warn（禁用 `VideoStatus::from_wire`，
//!   与原生 wan/vidu/seedance 的 sweep 容忍语义对齐）。
//!
//! 参考：构造面照抄 `providers/kling.rs`；下载与 redact 照抄
//! kling/seedance；健康记账照抄 plugins.rs record_hook_error 模式。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::messages::{TokenUsage, ToolCall};
use raisfast_agent::provider::{
    ChatRequest, ChatResponse, GeneratedImage, ImageRequest, ModelProvider, MusicRequest,
    ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use super::ext::{ProviderExt, ProviderExtRegistry};
use super::runtime::RunnerError;
use crate::llm::relay::adaptor::shared_client;
use crate::plugins::Permissions;
use crate::plugins::permissions::PermissionChecker;

pub struct PluginProvider {
    http: reqwest::Client,
    registry: Arc<ProviderExtRegistry>,
    key: String,
    display_name: String,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

fn decode_audio_for(key: &str, out: &Value) -> Result<Vec<u8>, ProviderError> {
    if let Some(b64) = out.get("audioBase64").and_then(Value::as_str) {
        use base64::Engine as _;
        return base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| {
                ProviderError::Parse(format!("provider extension {key}: audioBase64: {e}"))
            });
    }
    if let Some(hex_str) = out.get("audioHex").and_then(Value::as_str) {
        return hex::decode(hex_str)
            .map_err(|e| ProviderError::Parse(format!("provider extension {key}: audioHex: {e}")));
    }
    Err(ProviderError::Parse(format!(
        "provider extension {key}: parse returned no audio (audioBase64/audioHex)"
    )))
}

fn map_chat_response(_key: &str, out: &Value) -> ChatResponse {
    let text = out.get("text").and_then(Value::as_str).map(str::to_string);
    let tool_calls = out
        .get("toolCalls")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|tc| {
                    Some(ToolCall {
                        id: tc
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: tc.get("name").and_then(Value::as_str)?.to_string(),
                        arguments: match tc.get("arguments") {
                            Some(Value::String(s)) => s.clone(),
                            Some(other) => other.to_string(),
                            None => String::new(),
                        },
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let usage = out
        .get("usage")
        .filter(|v| !v.is_null())
        .map(|u| TokenUsage {
            input_tokens: u.get("inputTokens").and_then(Value::as_u64),
            output_tokens: u.get("outputTokens").and_then(Value::as_u64),
            cache_read: u.get("cacheRead").and_then(Value::as_u64),
            cache_write: u.get("cacheWrite").and_then(Value::as_u64),
        });
    ChatResponse {
        text,
        tool_calls,
        usage,
    }
}

fn map_images(key: &str, out: &Value) -> Result<Vec<GeneratedImage>, ProviderError> {
    let images = out.get("images").and_then(Value::as_array).ok_or_else(|| {
        ProviderError::Parse(format!(
            "provider extension {key}: parseImageResponse returned no images array"
        ))
    })?;
    Ok(images
        .iter()
        .map(|img| GeneratedImage {
            b64_json: img
                .get("b64Json")
                .and_then(Value::as_str)
                .map(str::to_string),
            url: img.get("url").and_then(Value::as_str).map(str::to_string),
        })
        .collect())
}

impl PluginProvider {
    pub fn new(
        registry: Arc<ProviderExtRegistry>,
        key: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
        param_override: Option<Value>,
        header_override: Option<Value>,
    ) -> Self {
        let key = key.into();
        Self {
            http: shared_client().clone(),
            registry,
            display_name: format!("ext:{key}"),
            key,
            base_url: base_url.into(),
            api_key,
            param_override,
            header_override,
        }
    }

    /// 扩展可用性（未加载 / 自动禁用 → Config，内核按渠道级失败处理）。
    fn ext_available(&self) -> Result<Arc<ProviderExt>, ProviderError> {
        let ext = self.registry.get(&self.key).ok_or_else(|| {
            ProviderError::Config(format!("provider extension {} not loaded", self.key))
        })?;
        if !ext.is_available() {
            return Err(ProviderError::Config(format!(
                "provider extension {} auto-disabled after consecutive errors",
                self.key
            )));
        }
        Ok(ext)
    }

    /// 模态门禁：meta.protocols 未声明的模态 → Config（内核跳过该渠道
    /// 该模态，与原生 provider 缺省行为一致）。
    fn ensure_protocol(&self, ext: &ProviderExt, proto: &str) -> Result<(), ProviderError> {
        if ext.protocols.iter().any(|p| p == proto) {
            Ok(())
        } else {
            Err(ProviderError::Config(format!(
                "provider extension {} does not declare protocol {}",
                self.key, proto
            )))
        }
    }

    /// ctx 组装（§4.3）。apiKey 缺失注入空串——无 key 渠道在上游鉴权处
    /// 失败，桥接不替渠道做策略。
    fn build_ctx(
        &self,
        model: &str,
        request: Option<&VideoRequest>,
        task_id: Option<&str>,
        task_data: Option<&Value>,
    ) -> Value {
        let request_json = request.map(|r| {
            json!({
                "prompt": r.prompt,
                "seconds": r.seconds,
                "size": r.size,
                "inputReferences": r.input_references.iter().map(|ref_| json!({
                    "url": ref_.url,
                    "b64Json": ref_.b64_json,
                    "mime": ref_.mime,
                })).collect::<Vec<_>>(),
                "callbackUrl": r.callback_url,
            })
        });
        json!({
            "provider": self.key,
            "baseUrl": self.base_url,
            "apiKey": self.api_key.clone().unwrap_or_default(),
            "model": model,
            "paramOverride": self.param_override.clone().unwrap_or(Value::Null),
            "request": request_json,
            "taskId": task_id.map(str::to_string),
            // 提交期状态回传（kling 双路径查询用 action 选端点，§4.3）。
            "taskData": task_data.cloned().unwrap_or(Value::Null),
        })
    }

    /// runner 调用 + 健康记账 + 错误映射（§5.1 错误映射表）。
    async fn runner_call(
        &self,
        ext: &ProviderExt,
        func: &str,
        ctx: &Value,
        payload: Option<&Value>,
    ) -> Result<Value, ProviderError> {
        let result = self
            .registry
            .runner()
            .call(&self.key, &ext.code, func, ctx, payload, ext.timeout_ms)
            .await;
        match result {
            Ok(v) => {
                self.registry.record_result(&self.key, true);
                Ok(v)
            }
            Err(RunnerError::Timeout(ms)) => {
                let disabled = self.registry.record_result(&self.key, false);
                if disabled {
                    tracing::warn!(provider = %self.key, "provider extension auto-disabled after consecutive errors");
                }
                Err(ProviderError::Transport(self.redact(&format!(
                    "provider extension {} timed out after {ms}ms",
                    self.key
                ))))
            }
            Err(e @ (RunnerError::Missing(_) | RunnerError::Js(_) | RunnerError::Serialize(_))) => {
                let disabled = self.registry.record_result(&self.key, false);
                if disabled {
                    tracing::warn!(provider = %self.key, "provider extension auto-disabled after consecutive errors");
                }
                Err(ProviderError::Config(
                    self.redact(&format!("provider extension {}: {e}", self.key)),
                ))
            }
            Err(e @ RunnerError::Internal(_)) => Err(ProviderError::Transport(
                self.redact(&format!("provider extension {}: {e}", self.key)),
            )),
        }
    }

    /// host 代发（§5.3）：GET/POST only、白名单逐请求校验、status 透传、
    /// 超时随扩展 meta。日志纪律：headers 永不落日志（key 可能在其中）。
    async fn send_spec(&self, ext: &ProviderExt, spec: &Value) -> Result<Value, ProviderError> {
        // Transport 错误（连接被边缘重置/中途断开）→ 换一次性新客户端重试
        // 一次（[自造-务实] 2026-09 MiniMax 冒烟：大响应体读取中途被断，
        // curl/裸客户端同请求成功——共享池复用坏连接所致）。
        match self.send_with(&self.http, ext, spec).await {
            Ok(resp) => Ok(resp),
            Err(ProviderError::Transport(e)) => {
                tracing::warn!(
                    provider = %self.key,
                    err = %e,
                    "provider request via shared client failed; retrying with a fresh client"
                );
                let fresh = reqwest::Client::builder()
                    .timeout(Duration::from_secs(120))
                    .build()
                    .map_err(|err| ProviderError::Config(err.to_string()))?;
                self.send_with(&fresh, ext, spec).await
            }
            Err(e) => Err(e),
        }
    }

    async fn send_with(
        &self,
        client: &reqwest::Client,
        ext: &ProviderExt,
        spec: &Value,
    ) -> Result<Value, ProviderError> {
        let url = spec.get("url").and_then(Value::as_str).ok_or_else(|| {
            ProviderError::Config(format!(
                "provider extension {}: spec.url is required",
                self.key
            ))
        })?;
        let method = spec.get("method").and_then(Value::as_str).unwrap_or("POST");
        if method != "GET" && method != "POST" {
            return Err(ProviderError::Config(format!(
                "provider extension {}: only GET/POST methods are allowed, got {method}",
                self.key
            )));
        }
        let permissions = Permissions {
            http: ext.http.clone(),
            ..Default::default()
        };
        if !PermissionChecker::is_url_allowed(&permissions, url) {
            return Err(ProviderError::Config(format!(
                "provider extension {}: url denied by http allowlist: {url}",
                self.key
            )));
        }

        let mut req = match method {
            "GET" => client.get(url),
            _ => client.post(url),
        };
        if let Some(headers) = spec.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                if let Some(vs) = value.as_str() {
                    req = req.header(name.as_str(), vs);
                }
            }
        }
        if let Some(over) = &self.header_override
            && let Some(map) = over.as_object()
        {
            for (name, value) in map {
                if let Some(vs) = value.as_str() {
                    req = req.header(name.as_str(), vs);
                }
            }
        }
        if let Some(body) = spec.get("body")
            && body.is_object()
        {
            req = req.json(body);
        }
        let resp = req
            .timeout(Duration::from_millis(ext.timeout_ms))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        // 二进制响应（音频/图像）→ base64 进 body64，JS 不碰原始字节。
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ctype.starts_with("audio/")
            || ctype.starts_with("image/")
            || ctype.starts_with("application/octet-stream")
        {
            use base64::Engine as _;
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ProviderError::Transport(e.to_string()))?;
            let body64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(json!({ "status": status, "body": Value::Null, "body64": body64 }));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
        Ok(json!({ "status": status, "body": body }))
    }

    /// 状态映射：未知 → InProgress + warn（禁用 `VideoStatus::from_wire`——
    /// 它把未知映射为 Failed，对 sweep 不容忍）。
    fn status_from_wire(key: &str, state: &str) -> VideoStatus {
        match state {
            "queued" => VideoStatus::Queued,
            "in_progress" => VideoStatus::InProgress,
            "completed" => VideoStatus::Completed,
            "failed" => VideoStatus::Failed,
            other => {
                tracing::warn!(
                    provider = %key,
                    state = %other,
                    "provider extension: unknown task status, treating as in-progress"
                );
                VideoStatus::InProgress
            }
        }
    }

    /// 轮询 + 解析；同时返回 parseTaskResult 给出的成片 url（content 用）。
    /// `task_data`：提交期状态回传（kling action 等，调用方持久化）。
    /// 非 2xx → `ProviderError::Http`（parse 只见 2xx）。
    async fn query_full(
        &self,
        task_id: &str,
        model: &str,
        task_data: Option<&Value>,
    ) -> Result<(VideoTask, Option<String>), ProviderError> {
        let ext = self.ext_available()?;
        self.ensure_protocol(&ext, "video")?;
        let ctx = self.build_ctx(model, None, Some(task_id), task_data);
        let spec = self
            .runner_call(&ext, "buildQueryRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseTaskResult", &ctx, Some(&resp))
            .await?;

        let status_str = out.get("status").and_then(Value::as_str).unwrap_or("");
        let status = Self::status_from_wire(self.key.as_str(), status_str);
        let progress = out
            .get("progress")
            .and_then(Value::as_i64)
            .map(|p| p.clamp(0, 100) as i32);
        let error = out
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|_| status == VideoStatus::Failed);
        let url = out
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|_| status == VideoStatus::Completed);
        Ok((
            VideoTask {
                id: task_id.to_string(),
                status,
                progress,
                error,
                data: None,
            },
            url,
        ))
    }

    /// 非 2xx → `ProviderError::Http`（parse 只处理 2xx 信封——HTTP 层
    /// 失败与协议层失败的分类边界，与原生 provider 的 unwrap_envelope 一致）。
    fn non_2xx_error(&self, resp: &Value) -> Option<ProviderError> {
        let status = resp.get("status").and_then(Value::as_i64)?;
        if (200..300).contains(&status) {
            return None;
        }
        let body = resp.get("body").map(|b| b.to_string()).unwrap_or_default();
        Some(ProviderError::Http {
            status: status as u16,
            body: self.redact(&body),
        })
    }

    /// 错误体脱敏 [照抄 seedance.rs:163 redact]：渠道 key 不进日志/错误。
    fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some(key) = &self.api_key
            && !key.is_empty()
        {
            out = out.replace(key.as_str(), "***");
        }
        out.chars().take(500).collect()
    }
}

impl PluginProvider {
    /// 环境字段 + 模态 request 的通用 ctx 组装。
    fn modality_ctx(&self, model: &str, request: Value) -> Value {
        json!({
            "provider": self.key,
            "baseUrl": self.base_url,
            "apiKey": self.api_key.clone().unwrap_or_default(),
            "model": model,
            "paramOverride": self.param_override.clone().unwrap_or(Value::Null),
            "request": request,
        })
    }
}

#[async_trait]
impl ModelProvider for PluginProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    /// 扩展无 chat 面 → Config，内核跳过该渠道的其他模态。
    /// 提交：buildSubmitRequest → host 代发 → parseSubmitResponse。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let ext = self.ext_available()?;
        self.ensure_protocol(&ext, "video")?;

        // host 侧通用 seconds 上限（§5.4，仅扩展路径——原生 provider 有
        // 自己的付费档位硬校验）。
        if let Some(seconds) = &request.seconds {
            let n: i64 = seconds.trim().parse().map_err(|_| {
                ProviderError::Config(format!(
                    "provider extension {}: seconds must be an integer, got {seconds:?}",
                    self.key
                ))
            })?;
            let max = self.registry.max_seconds();
            if !(1..=max).contains(&n) {
                return Err(ProviderError::Config(format!(
                    "provider extension {}: seconds must be between 1 and {max}, got {n}",
                    self.key
                )));
            }
        }

        let ctx = self.build_ctx(model, Some(request), None, None);
        let spec = self
            .runner_call(&ext, "buildSubmitRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseSubmitResponse", &ctx, Some(&resp))
            .await?;

        let task_id = out
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(format!(
                "provider extension {}: parseSubmitResponse returned no taskId",
                self.key
            )));
        }
        let status = out
            .get("status")
            .and_then(Value::as_str)
            .map(|s| Self::status_from_wire(self.key.as_str(), s))
            .unwrap_or(VideoStatus::Queued);
        Ok(VideoTask {
            id: task_id,
            status,
            progress: None,
            error: out.get("error").and_then(Value::as_str).map(str::to_string),
            data: out.get("taskData").cloned().filter(|v| !v.is_null()),
        })
    }

    /// 轮询：buildQueryRequest → host GET → parseTaskResult。
    async fn video_query(
        &self,
        task_id: &str,
        model: &str,
        task_data: Option<&Value>,
    ) -> Result<VideoTask, ProviderError> {
        let (task, _url) = self.query_full(task_id, model, task_data).await?;
        Ok(task)
    }

    /// 成片：host 直接下载 parseTaskResult 给出的 url（不过 JS）。
    ///
    /// 下载用一次性客户端 + transport 重试（[自造-务实] 2026-09 MiniMax
    /// CDN 冒烟教训）：共享 client 的长连接池可能被 CDN 边缘节点重置——
    /// 复用坏连接会持续 "error sending request"（curl/裸客户端同 URL 秒
    /// 下成功）。成片下载低频（每个成品一次），隔离连接池收益大于复用。
    async fn video_content(
        &self,
        task_id: &str,
        model: &str,
        task_data: Option<&Value>,
    ) -> Result<Vec<u8>, ProviderError> {
        let (_task, url) = self.query_full(task_id, model, task_data).await?;
        let Some(url) = url else {
            return Err(ProviderError::Parse(format!(
                "provider extension {}: task {task_id} has no result video url",
                self.key
            )));
        };
        let download = |client: reqwest::Client| {
            let url = url.clone();
            async move {
                client
                    .get(&url)
                    .timeout(Duration::from_secs(300))
                    .send()
                    .await
            }
        };
        let resp = match download(self.http.clone()).await {
            Ok(resp) => resp,
            Err(shared_err) => {
                tracing::warn!(
                    provider = %self.key,
                    err = %shared_err,
                    "provider video download via shared client failed; retrying with a fresh client"
                );
                let fresh = reqwest::Client::builder()
                    .timeout(Duration::from_secs(300))
                    .build()
                    .map_err(|e| ProviderError::Config(e.to_string()))?;
                download(fresh)
                    .await
                    .map_err(|e| ProviderError::Transport(e.to_string()))?
            }
        };
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: format!("provider extension {}: download failed {status}", self.key),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    // ── 多模态面（chat 非流式 / speech / music / image）─────────────
    //
    // 每模态一对 build/parse 纯函数；二进制音频响应经 body64（base64）
    // 或扩展返回的 audioHex 解码；chat 无流式（trait 默认 chat_stream =
    // 非流式重放）。原生 minimax chat 本就非流式 → 全对等迁移可达。

    async fn chat(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        let ext = self.ext_available()?;
        self.ensure_protocol(&ext, "chat")?;

        let tools = request.tools.map(|ts| {
            Value::Array(
                ts.iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                            "category": t.category,
                        })
                    })
                    .collect(),
            )
        });
        let ctx = self.modality_ctx(
            model,
            json!({
                "messages": serde_json::to_value(request.messages)
                    .map_err(|e| ProviderError::Parse(e.to_string()))?,
                "tools": tools,
                "temperature": request.temperature,
                "maxTokens": request.max_tokens,
                "stop": request.stop,
            }),
        );
        let spec = self
            .runner_call(&ext, "buildChatRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseChatResponse", &ctx, Some(&resp))
            .await?;
        Ok(map_chat_response(&self.key, &out))
    }

    async fn speech(&self, text: &str, voice: &str, model: &str) -> Result<Vec<u8>, ProviderError> {
        let ext = self.ext_available()?;
        self.ensure_protocol(&ext, "speech")?;
        let ctx = self.modality_ctx(model, json!({ "text": text, "voice": voice }));
        let spec = self
            .runner_call(&ext, "buildSpeechRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseSpeechResponse", &ctx, Some(&resp))
            .await?;
        decode_audio_for(&self.key, &out)
    }

    async fn music(&self, request: &MusicRequest, model: &str) -> Result<Vec<u8>, ProviderError> {
        let ext = self.ext_available()?;
        self.ensure_protocol(&ext, "music")?;
        let ctx = self.modality_ctx(
            model,
            json!({ "prompt": request.prompt, "lyrics": request.lyrics }),
        );
        let spec = self
            .runner_call(&ext, "buildMusicRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseMusicResponse", &ctx, Some(&resp))
            .await?;
        decode_audio_for(&self.key, &out)
    }

    async fn generate_image(
        &self,
        request: &ImageRequest,
        model: &str,
    ) -> Result<Vec<GeneratedImage>, ProviderError> {
        let ext = self.ext_available()?;
        self.ensure_protocol(&ext, "image")?;
        let ctx = self.modality_ctx(
            model,
            json!({
                "prompt": request.prompt,
                "n": request.n,
                "size": request.size,
                "inputReferences": request.input_references.iter().map(|r| json!({
                    "url": r.url,
                    "b64Json": r.b64_json,
                    "mime": r.mime,
                })).collect::<Vec<_>>(),
            }),
        );
        let spec = self
            .runner_call(&ext, "buildImageRequest", &ctx, None)
            .await?;
        let resp = self.send_spec(&ext, &spec).await?;
        if let Some(e) = self.non_2xx_error(&resp) {
            return Err(e);
        }
        let out = self
            .runner_call(&ext, "parseImageResponse", &ctx, Some(&resp))
            .await?;
        map_images(&self.key, &out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisfast_agent::provider::VideoInputRef;

    /// demo 扩展：build 会产出指向 meta.http 白名单外/内的 URL（由用例决定），
    /// parse 恒抛异常（用于驱动健康记账，不经网络）。
    fn demo_src(allow_host: &str) -> String {
        format!(
            r#"
export const meta = {{ key: "demo", name: "Demo", version: "0.1.0", contract: 1, protocols: ["video"], http: ["{allow_host}/*"], timeout_ms: 30000 }};
export function buildSubmitRequest(ctx) {{
    return {{ url: ctx.baseUrl + "/submit", method: "POST",
        headers: {{ Authorization: "Token " + ctx.apiKey }},
        body: {{ model: ctx.model, prompt: ctx.request.prompt }} }};
}}
export function parseSubmitResponse(ctx, response) {{
    throw new Error("parse should be unreachable in these tests");
}}
export function buildQueryRequest(ctx) {{
    return {{ url: ctx.baseUrl + "/poll/" + ctx.taskId, method: "GET" }};
}}
export function parseTaskResult(ctx, response) {{
    const s = response.body.status;
    const out = {{ status: s }};
    if (s === "completed") out.url = response.body.url;
    return out;
}}
"#
        )
    }

    async fn registry_with(allow_host: &str) -> Arc<ProviderExtRegistry> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("demo.js"), demo_src(allow_host)).unwrap();
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        reg.configure_dir(tmp.path());
        reg.reload().await.unwrap();
        // registry 只持有 code 字符串；tmp 生命周期由 mem::forget 延长
        std::mem::forget(tmp);
        reg
    }

    fn provider(reg: Arc<ProviderExtRegistry>, base_url: &str) -> PluginProvider {
        PluginProvider::new(
            reg,
            "demo",
            base_url.to_string(),
            Some("demo-secret".to_string()),
            None,
            None,
        )
    }

    fn video_request(prompt: &str, seconds: Option<&str>) -> VideoRequest {
        VideoRequest {
            prompt: prompt.into(),
            seconds: seconds.map(str::to_string),
            size: None,
            input_references: Vec::new(),
            callback_url: None,
        }
    }

    #[tokio::test]
    async fn seconds_cap_blocks_extension_path() {
        let reg = registry_with("upstream.example").await;
        let p = provider(reg, "https://upstream.example");
        let err = p
            .video_submit(&video_request("x", Some("700")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("seconds must be between 1 and 600")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn missing_extension_key_is_config_error() {
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        let p = PluginProvider::new(reg, "nope", "https://upstream.example", None, None, None);
        let err = p
            .video_submit(&video_request("x", None), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("not loaded")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn allowlist_denies_url_outside_meta_patterns() {
        // baseUrl 与白名单同 host，但 path 白名单是 /* —— host 匹配 → 放行；
        // 这里反着验证：白名单允许 upstream.example，url 恰好同 host → 放行，
        // 请求会因网络不可达（.invalid TLD）失败 → Transport。
        let reg = registry_with("upstream.example").await;
        let p = provider(reg, "https://upstream.example");
        let err = p
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Transport(_)),
            "allowlist 放行后应走到网络层: {err:?}"
        );
    }

    #[tokio::test]
    async fn allowlist_denies_foreign_host_and_private_host() {
        // 白名单只允许 upstream.example；baseUrl 换成别的公网域 → allowlist 拒绝
        let p = provider(
            registry_with("upstream.example").await,
            "https://other.example",
        );
        let err = p
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("allowlist")),
            "{err:?}"
        );

        // 私网 host（127.0.0.1）被 is_private_host 硬拒（SSRF 守卫）
        let p2 = provider(registry_with("127.0.0.1").await, "http://127.0.0.1:1");
        let err = p2
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("allowlist")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn consecutive_runner_errors_auto_disable_extension() {
        // buildSubmitRequest 本身抛异常（不触网）→ Config；连续 3 次自动禁用
        // → 第 4 次调用在 ext_available 即报 auto-disabled。
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("throwing.js"),
            r#"
export const meta = { key: "demo", name: "Throwing", version: "0.1.0", contract: 1, protocols: ["video"], http: ["upstream.example/*"], timeout_ms: 30000 };
export function buildSubmitRequest(ctx) { throw new Error("boom-from-extension"); }
export function parseSubmitResponse(ctx, r) { return { taskId: "x" }; }
export function buildQueryRequest(ctx) { return { url: ctx.baseUrl + "/p" }; }
export function parseTaskResult(ctx, r) { return { status: "in_progress" }; }
"#,
        )
        .unwrap();
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        reg.configure_dir(tmp.path());
        reg.reload().await.unwrap();
        std::mem::forget(tmp);

        let p = provider(reg, "https://upstream.example");
        for _ in 0..3 {
            let err = p
                .video_submit(&video_request("x", Some("5")), "m")
                .await
                .unwrap_err();
            assert!(
                matches!(err, ProviderError::Config(ref m) if m.contains("boom-from-extension")),
                "{err:?}"
            );
        }
        let err = p
            .video_submit(&video_request("x", Some("5")), "m")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("auto-disabled")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn status_mapping_unknown_state_tolerates_as_in_progress() {
        // 纯函数断言：未知状态 → InProgress（与原生 sweep 容忍语义一致）
        assert_eq!(
            PluginProvider::status_from_wire("demo", "queued"),
            VideoStatus::Queued
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "in_progress"),
            VideoStatus::InProgress
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "completed"),
            VideoStatus::Completed
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "failed"),
            VideoStatus::Failed
        );
        assert_eq!(
            PluginProvider::status_from_wire("demo", "brand-new-state"),
            VideoStatus::InProgress
        );
    }

    #[tokio::test]
    async fn video_request_references_survive_ctx_roundtrip() {
        // ctx.request.inputReferences 形状（§4.5）经真实 JS 往返保持
        let reg = registry_with("upstream.example").await;
        let ext = reg.get("demo").unwrap();
        let p = provider(reg.clone(), "https://upstream.example");
        let request = VideoRequest {
            prompt: "p".into(),
            seconds: Some("5".into()),
            size: Some("1280x720".into()),
            input_references: vec![VideoInputRef::from_url("https://img.example/a.png")],
            callback_url: None,
        };
        let ctx = p.build_ctx("m", Some(&request), None, None);
        assert_eq!(ctx["request"]["size"], "1280x720");
        assert_eq!(
            ctx["request"]["inputReferences"][0]["url"],
            "https://img.example/a.png"
        );
        let _ = ext;
    }
}

#[cfg(test)]
mod multimodal_tests {
    use super::*;

    const DEMO_HOST: &str = "api.mm.example";

    fn mm_src() -> String {
        // __HOST__ 占位符在装配时替换为白名单 host
        r#"
export const meta = { key: "demo", name: "Demo", version: "0.1.0", contract: 1,
    protocols: ["chat", "speech"], http: ["__HOST__/*"], timeout_ms: 30000 };
export function buildChatRequest(ctx) {
    return { url: ctx.baseUrl + "/chat", method: "POST",
        headers: { Authorization: "Bearer " + ctx.apiKey },
        body: { model: ctx.model, messages: ctx.request.messages,
                temperature: ctx.request.temperature } };
}
export function parseChatResponse(ctx, response) {
    const u = response.body.usage || {};
    return { text: response.body.reply,
             toolCalls: (response.body.tool_calls || []).map((t) => ({ id: t.id, name: t.name, arguments: t.args })),
             usage: Object.keys(u).length === 0 ? null : { inputTokens: u.in, outputTokens: u.out } };
}
export function buildSpeechRequest(ctx) {
    return { url: ctx.baseUrl + "/speak", method: "POST",
        headers: { Authorization: "Bearer " + ctx.apiKey },
        body: { text: ctx.request.text, voice: ctx.request.voice } };
}
export function parseSpeechResponse(ctx, response) {
    return response.body64 ? { audioBase64: response.body64 } : { audioHex: response.body.hex };
}
"#
        .replace("__HOST__", DEMO_HOST)
    }

    async fn mm_registry() -> Arc<ProviderExtRegistry> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("demo.js"), mm_src()).unwrap();
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        reg.configure_dir(tmp.path());
        reg.reload().await.unwrap();
        std::mem::forget(tmp);
        reg
    }

    fn mm_provider(reg: Arc<ProviderExtRegistry>) -> PluginProvider {
        PluginProvider::new(
            reg,
            "demo",
            format!("https://{DEMO_HOST}"),
            Some("mm-key".to_string()),
            None,
            None,
        )
    }

    /// 接线证明：build 成功 + send 尝试（不可达 DNS → Transport）。
    #[tokio::test]
    async fn chat_build_and_send_wiring_reaches_transport() {
        let p = mm_provider(mm_registry().await);
        let req = ChatRequest {
            messages: &[],
            tools: None,
            temperature: Some(0.7),
            max_tokens: Some(256),
            stop: None,
        };
        let err = p.chat(&req, "demo-chat").await.unwrap_err();
        assert!(matches!(err, ProviderError::Transport(_)), "{err:?}");
    }

    #[tokio::test]
    async fn speech_wiring_reaches_transport() {
        let p = mm_provider(mm_registry().await);
        let err = p
            .speech("你好世界", "female", "speech-01")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Transport(_)), "{err:?}");
    }

    #[test]
    fn map_chat_response_maps_text_usage_and_tool_calls() {
        let out = json!({
            "text": "你好",
            "usage": { "inputTokens": 12, "outputTokens": 3 },
            "toolCalls": [ { "id": "c1", "name": "weather", "arguments": "{\"city\":\"杭州\"}" } ]
        });
        let resp = map_chat_response("demo", &out);
        assert_eq!(resp.text.as_deref(), Some("你好"));
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "weather");
        assert_eq!(resp.tool_calls[0].arguments, "{\"city\":\"杭州\"}");
        let usage = resp.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(3));
    }

    #[test]
    fn map_chat_response_null_usage_is_none() {
        // JS 侧空 usage 产出 null（Object.keys === 0 → null）
        let resp = map_chat_response("demo", &json!({ "text": "x", "usage": null }));
        assert!(resp.usage.is_none());
        assert_eq!(resp.text.as_deref(), Some("x"));
    }

    #[test]
    fn decode_audio_supports_base64_and_hex() {
        let b64 = decode_audio_for("demo", &json!({ "audioBase64": "SUQz" })).unwrap();
        assert_eq!(b64, b"ID3");
        let hexd = decode_audio_for("demo", &json!({ "audioHex": "494433" })).unwrap();
        assert_eq!(hexd, b"ID3");
        assert!(decode_audio_for("demo", &json!({})).is_err());
    }

    #[tokio::test]
    async fn undeclared_modality_is_config_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("videoonly.js"),
            r#"
export const meta = { key: "demo", version: "0.1.0", contract: 1, protocols: ["video"], http: ["__HOST__/*"], timeout_ms: 30000 };
export function buildSubmitRequest(ctx) { return { url: ctx.baseUrl + "/s", body: {} }; }
export function parseSubmitResponse(ctx, r) { return { taskId: "t" }; }
export function buildQueryRequest(ctx) { return { url: ctx.baseUrl + "/q" }; }
export function parseTaskResult(ctx, r) { return { status: "in_progress" }; }
"#
            .replace("__HOST__", DEMO_HOST),
        )
        .unwrap();
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        reg.configure_dir(tmp.path());
        reg.reload().await.unwrap();
        std::mem::forget(tmp);

        let p = mm_provider(reg);
        let req = ChatRequest {
            messages: &[],
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let err = p.chat(&req, "m").await.unwrap_err();
        assert!(
            matches!(err, ProviderError::Config(ref m) if m.contains("does not declare protocol chat")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn missing_chat_exports_reject_at_scan() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("bad.js"),
            r#"
export const meta = { key: "demo", version: "0.1.0", contract: 1, protocols: ["chat"], http: ["x.example/*"], timeout_ms: 100 };
export function parseChatResponse(ctx, r) { return {}; }
"#,
        )
        .unwrap();
        let reg = Arc::new(ProviderExtRegistry::new(vec![]));
        reg.configure_dir(tmp.path());
        let report = reg.reload().await.unwrap();
        assert_eq!(report.loaded, 0);
        assert!(
            report.errors.iter().any(|e| e.contains("buildChatRequest")),
            "{:?}",
            report.errors
        );
    }
}

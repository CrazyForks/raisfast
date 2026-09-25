//! MiniMax Hailuo provider (official API) for internal consumption: video
//! surface over MiniMax's V1/V2 video-generation protocols —
//! `provider: "minimax"` channels route onto `LlmRouter::call().video_*`.
//!
//! Reference matrix:
//! - protocol (V2 H3 multimodal `content[]` vs V1 flat fields, dual submit/
//!   query paths, per-model duration & resolution rules, status vocabularies,
//!   `base_resp` reject envelope, `error.{code,message}` API errors,
//!   file_id authed download, H3 public CDN url, callback_url):
//!   [照抄 new-api `plugins/tasks/hailuo/plugin.js` — vendored, symbol-level].
//! - auth: `Authorization: Bearer <api_key>`, **no GroupId** [照抄 plugin
//!   buildSubmitRequest/buildQueryRequest/buildContentRequest].
//! - model routing: `MiniMax-H3` → V2 (multimodal content, ratio enum,
//!   duration INT 4..15 **validated not clamped**, resolution {768P,2K}
//!   default 768P); everything else → V1 (flat fields, duration default 6,
//!   resolutionFor mapping 2.3/02→768P else 720P; 01-series duration 6).
//! - i2v: H3 via `content` `image_url` entries (references); V1 via
//!   `first_frame_image` (first reference).
//! - deviations (flagged): [自造-适配] unknown query status → `InProgress`
//!   (distributed sweep + deadline gate converge, vs the plugin's UNKNOWN
//!   bucket); H3 duration validates instead of clamping [照抄 plugin
//!   h3Duration 原行为].

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::messages::{TokenUsage, ToolCall};
use raisfast_agent::provider::openai::wire_chat_body;
use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use crate::llm::relay::adaptor::shared_client;

const H3_MODEL: &str = "MiniMax-H3";
/// [照抄 plugin H3_MIN/H3_MAX_DURATION = 4..15, H3_DEFAULT_DURATION = 5].
const H3_MIN_DURATION: i64 = 4;
const H3_MAX_DURATION: i64 = 15;
const H3_DEFAULT_DURATION: i64 = 5;
const H3_RATIOS: [&str; 7] = ["adaptive", "21:9", "16:9", "4:3", "1:1", "3:4", "9:16"];

pub struct MiniMaxProvider {
    http: reqwest::Client,
    base_url: String,
    /// `(api_key, Option<group_id>)` — key format `api_key:group_id`
    /// [照抄 kling.rs 双段组合约定]：video 端点只用 api_key（Group 无关），
    /// TTS `t2a_v2` 强制要求 GroupId 查询参数。单段 key = 仅视频可用。
    credential: Option<(String, Option<String>)>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

/// H3 vs V1 — the two generations speak different contracts.
fn is_h3(model: &str) -> bool {
    model == H3_MODEL
}

impl MiniMaxProvider {
    /// `base_url` is the OFFICIAL root WITHOUT a version segment
    /// (e.g. `https://api.minimaxi.com`) — `/v1/*` and `/v2/*` both hang
    /// off it [照抄 plugin：两个代际路径共用同一 root]。
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        param_override: Option<Value>,
        header_override: Option<Value>,
    ) -> Self {
        let credential = api_key.map(|k| match k.split_once(':') {
            Some((key, gid)) => (key.trim().to_string(), Some(gid.trim().to_string())),
            None => (k.trim().to_string(), None),
        });
        Self {
            http: shared_client().clone(),
            base_url: base_url.into(),
            credential,
            param_override,
            header_override,
        }
    }

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let key = self.credential.as_ref().map(|(k, _)| k.clone());
        let mut req = req.bearer_auth(key.unwrap_or_default());
        if let Some(over) = &self.header_override
            && let Some(map) = over.as_object()
        {
            for (k, v) in map {
                if let Some(vs) = v.as_str() {
                    req = req.header(k.as_str(), vs);
                }
            }
        }
        req
    }

    /// 错误体脱敏：上游 key 不进日志/错误消息（providers 域纪律）。
    fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some((key, gid)) = &self.credential {
            for secret in [key.as_str(), gid.clone().unwrap_or_default().as_str()] {
                if !secret.is_empty() {
                    out = out.replace(secret, "***");
                }
            }
        }
        out.chars().take(500).collect()
    }

    fn override_value(&self, key: &str) -> Option<&str> {
        self.param_override
            .as_ref()
            .and_then(|o| o.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// Shallow-merge channel `param_override` (skipping keys the node owns).
    fn merge_overrides(&self, body: &mut Value, skip: &[&str]) {
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object)
            && let Some(target) = body.as_object_mut()
        {
            for (k, v) in map {
                if !skip.contains(&k.as_str()) {
                    target.insert(k.clone(), v.clone());
                }
            }
        }
    }

    // ── H3 (V2) helpers [照抄 plugin h3Duration/h3Resolution/h3Ratio] ──

    fn h3_duration(&self, request: &VideoRequest) -> Result<i64, ProviderError> {
        let raw = request
            .seconds
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(raw) = raw else {
            return Ok(H3_DEFAULT_DURATION);
        };
        let seconds: i64 = raw
            .parse()
            .map_err(|_| {
                ProviderError::Config(format!(
                    "{H3_MODEL} duration must be an integer between {H3_MIN_DURATION} and {H3_MAX_DURATION} seconds"
                ))
            })?;
        if !(H3_MIN_DURATION..=H3_MAX_DURATION).contains(&seconds) {
            return Err(ProviderError::Config(format!(
                "{H3_MODEL} duration must be an integer between {H3_MIN_DURATION} and {H3_MAX_DURATION} seconds"
            )));
        }
        Ok(seconds)
    }

    fn h3_resolution(&self, request: &VideoRequest) -> Result<String, ProviderError> {
        let raw = request
            .size
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_uppercase)
            .or_else(|| self.override_value("resolution").map(str::to_uppercase));
        let Some(raw) = raw else {
            return Ok("768P".to_string());
        };
        if raw.contains("2K") {
            return Ok("2K".into());
        }
        if raw.contains("768") {
            return Ok("768P".into());
        }
        Err(ProviderError::Config(format!(
            "{H3_MODEL} resolution must be 768P or 2K"
        )))
    }

    fn h3_ratio(&self, _request: &VideoRequest, has_visual: bool) -> Result<String, ProviderError> {
        let ratio = self
            .override_value("ratio")
            .map(str::to_string)
            .unwrap_or_else(|| {
                if has_visual {
                    "adaptive".to_string()
                } else {
                    "16:9".to_string()
                }
            });
        if !H3_RATIOS.contains(&ratio.as_str()) {
            return Err(ProviderError::Config(format!(
                "{H3_MODEL} ratio must be one of {}",
                H3_RATIOS.join(", ")
            )));
        }
        if ratio == "adaptive" && !has_visual {
            return Err(ProviderError::Config(format!(
                "{H3_MODEL} ratio adaptive requires an image or video input"
            )));
        }
        Ok(ratio)
    }

    // ── V1 helpers [照抄 plugin outboundDuration/resolutionFor] ──

    fn v1_duration(&self, request: &VideoRequest) -> i64 {
        request
            .seconds
            .as_deref()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(6)
    }

    /// [照抄 plugin `resolutionFor` + `defaultResolution`].
    fn v1_resolution(&self, request: &VideoRequest, model: &str) -> String {
        let raw = request
            .size
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| self.override_value("resolution").map(str::to_string));
        let Some(raw) = raw else {
            return self.v1_default_resolution(model);
        };
        if raw.contains("1080") {
            return "1080P".into();
        }
        if raw.contains("768") {
            return "768P".into();
        }
        if raw.contains("720") {
            return if is_modern_hailuo(model) {
                "768P".into()
            } else {
                "720P".into()
            };
        }
        if raw.contains("512") {
            return "512P".into();
        }
        self.v1_default_resolution(model)
    }

    fn v1_default_resolution(&self, model: &str) -> String {
        if matches!(
            model,
            "MiniMax-Hailuo-2.3" | "MiniMax-Hailuo-2.3-Fast" | "MiniMax-Hailuo-02"
        ) {
            "768P".into()
        } else {
            "720P".into()
        }
    }

    // ── transport ──

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, ProviderError> {
        let resp = self
            .apply_headers(req)
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: self.redact(text.lines().next().unwrap_or_default()),
            });
        }
        serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {}", self.redact(&text))))
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        self.send(self.http.post(&url).json(body)).await
    }

    async fn get_json(&self, path: &str) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        self.send(self.http.get(&url)).await
    }

    /// 提交/查询共用的错误判定：`base_resp.status_code != 0`（V1 包裹层）或
    /// 顶层 `error.{code,message}`（API 错误）→ 确定性失败。
    fn check_envelope(&self, parsed: &Value) -> Result<(), ProviderError> {
        if let Some(err) = parsed.get("error").filter(|e| e.is_object()) {
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !message.is_empty() {
                let code = err
                    .get("http_code")
                    .or_else(|| err.get("code"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                return Err(ProviderError::Http {
                    status: u16::try_from(code).unwrap_or(500),
                    body: self.redact(message),
                });
            }
        }
        let code = parsed
            .pointer("/base_resp/status_code")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if code != 0 {
            let msg = parsed
                .pointer("/base_resp/status_msg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(ProviderError::Http {
                status: 500,
                body: self.redact(&format!("minimax code {code}: {msg}")),
            });
        }
        Ok(())
    }

    /// H3 任务体：query 响应含 `task` 对象（V2 独有包裹）。
    fn h3_task<'a>(&self, parsed: &'a Value) -> Option<&'a Value> {
        parsed.get("task").filter(|t| t.is_object())
    }

    fn status_from_wire(&self, parsed: &Value, h3: bool) -> VideoStatus {
        if h3 {
            // submit 响应 status 在顶层；query 响应包在 `task` 里 [照抄 plugin
            // parseSubmitResponse / h3QueryTask 的两处形态]。
            let status = self
                .h3_task(parsed)
                .and_then(|t| t.get("status"))
                .or_else(|| parsed.get("status"))
                .and_then(Value::as_str);
            match status {
                Some("succeeded") => VideoStatus::Completed,
                Some("failed") | Some("cancelled") => VideoStatus::Failed,
                Some("queued") => VideoStatus::Queued,
                _ => VideoStatus::InProgress,
            }
        } else {
            match parsed.get("status").and_then(Value::as_str) {
                Some("Success") => VideoStatus::Completed,
                Some("Fail") => VideoStatus::Failed,
                // Preparing/Queueing/Processing [照抄 plugin statuses 表].
                Some("Preparing") | Some("Queueing") | Some("Processing") => {
                    VideoStatus::InProgress
                }
                _ => VideoStatus::InProgress,
            }
        }
    }
}

fn is_modern_hailuo(model: &str) -> bool {
    matches!(
        model,
        "MiniMax-Hailuo-2.3" | "MiniMax-Hailuo-2.3-Fast" | "MiniMax-Hailuo-02"
    )
}

/// OpenAI-shaped chat response → agent `ChatResponse`（MiniMax chat wire 与
/// OpenAI 同形：`choices[0].message.{content,tool_calls}` + `usage`）。
fn parse_chat_response(parsed: &Value) -> Result<ChatResponse, ProviderError> {
    let choice = parsed
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .ok_or_else(|| ProviderError::Parse("minimax: chat response without choices".into()))?;
    let message = choice.get("message");
    let text = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let tool_calls: Vec<ToolCall> = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|c| {
                    Some(ToolCall {
                        id: c.get("id")?.as_str()?.to_string(),
                        name: c.pointer("/function/name")?.as_str()?.to_string(),
                        arguments: c
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let usage = parsed.get("usage").map(|u| TokenUsage {
        input_tokens: u.get("prompt_tokens").and_then(Value::as_u64),
        output_tokens: u.get("completion_tokens").and_then(Value::as_u64),
        cache_read: None,
        cache_write: None,
    });
    Ok(ChatResponse {
        text,
        tool_calls,
        usage,
    })
}

#[async_trait]
impl ModelProvider for MiniMaxProvider {
    fn name(&self) -> &str {
        "minimax"
    }

    /// 对话：`POST {base}/v1/text/chatcompletion_v2`——OpenAI wire 仅路径
    /// 不同 [照抄 new-api `relay/channel/minimax/relay-minimax.go` + 本仓
    /// providers/anthropic.rs 的 `wire_chat_body` 复用范式]。流式走 trait
    /// 默认（非流式单批回放），与「egress 非流式先行」先例一致。
    async fn chat(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        let body = wire_chat_body(request, model, false);
        let url = format!(
            "{}/v1/text/chatcompletion_v2",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .apply_headers(self.http.post(&url).json(&body))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: self.redact(text.lines().next().unwrap_or_default()),
            });
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        self.check_envelope(&parsed)?;
        parse_chat_response(&parsed)
    }

    /// 文生语音：`POST {base}/v1/t2a_v2?GroupId=…`（TTS 强制 GroupId——
    /// key 需 `api_key:group_id` 组合格式）。响应 `data.audio` 为
    /// **hex 编码**音频，解码为字节返回。计费 input-side 字符估算
    /// （facade `speech()` 语义）。
    async fn speech(&self, text: &str, voice: &str, model: &str) -> Result<Vec<u8>, ProviderError> {
        let Some((key, group)) = &self.credential else {
            return Err(ProviderError::Config(
                "minimax: missing api key (format `api_key:group_id`)".into(),
            ));
        };
        let Some(group) = group else {
            return Err(ProviderError::Config(
                "minimax: TTS requires key format `api_key:group_id` (GroupId is mandatory on t2a_v2)".into(),
            ));
        };
        let mut url = format!("{}/v1/t2a_v2", self.base_url.trim_end_matches('/'));
        url.push_str(&format!("?GroupId={}", urlencoding::encode(group)));

        let mut body = json!({
            "model": model,
            "text": text,
            "voice_setting": { "voice_id": voice },
            "audio_setting": {
                "format": "mp3",
                "sample_rate": 32000,
                "bitrate": 128000,
                "channel": 1,
            },
        });
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object) {
            for (k, v) in map {
                body[k.clone()] = v.clone();
            }
        }
        let mut req = self.http.post(&url).json(&body);
        req = req.bearer_auth(key);
        if let Some(over) = &self.header_override
            && let Some(map) = over.as_object()
        {
            for (k, v) in map {
                if let Some(vs) = v.as_str() {
                    req = req.header(k.as_str(), vs);
                }
            }
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: self.redact(text.lines().next().unwrap_or_default()),
            });
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {}", self.redact(&text))))?;
        self.check_envelope(&parsed)?;
        let audio_hex = parsed
            .pointer("/data/audio")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::Parse("minimax: speech response without data.audio".into())
            })?;
        let bytes = hex::decode(audio_hex.trim())
            .map_err(|e| ProviderError::Parse(format!("minimax: audio hex decode: {e}")))?;
        if bytes.is_empty() {
            return Err(ProviderError::Parse("minimax: empty audio".into()));
        }
        Ok(bytes)
    }

    /// 音乐生成：`POST {base}/v1/music_generation?GroupId=…`（与 TTS 同样
    /// 强制 GroupId）。响应 `data.audio` 为 hex 编码音频。同步接口
    /// [参考 MiniMax 公开 API 文档，本地无 vendored 参考]。
    async fn music(
        &self,
        request: &raisfast_agent::provider::MusicRequest,
        model: &str,
    ) -> Result<Vec<u8>, ProviderError> {
        let Some((key, group)) = &self.credential else {
            return Err(ProviderError::Config(
                "minimax: missing api key (format `api_key:group_id`)".into(),
            ));
        };
        let Some(group) = group else {
            return Err(ProviderError::Config(
                "minimax: music requires key format `api_key:group_id` (GroupId is mandatory on music_generation)".into(),
            ));
        };
        let mut url = format!(
            "{}/v1/music_generation",
            self.base_url.trim_end_matches('/')
        );
        url.push_str(&format!("?GroupId={}", urlencoding::encode(group)));

        let mut body = json!({
            "model": model,
            "prompt": request.prompt,
            "audio_setting": {
                "format": "mp3",
                "sample_rate": 32000,
                "bitrate": 128000,
                "channel": 1,
            },
        });
        if let Some(lyrics) = &request.lyrics
            && !lyrics.trim().is_empty()
        {
            body["lyrics"] = Value::String(lyrics.clone());
        }
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object) {
            for (k, v) in map {
                body[k.clone()] = v.clone();
            }
        }
        let mut req = self.http.post(&url).json(&body);
        req = req.bearer_auth(key);
        if let Some(over) = &self.header_override
            && let Some(map) = over.as_object()
        {
            for (k, v) in map {
                if let Some(vs) = v.as_str() {
                    req = req.header(k.as_str(), vs);
                }
            }
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: self.redact(text.lines().next().unwrap_or_default()),
            });
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {}", self.redact(&text))))?;
        self.check_envelope(&parsed)?;
        let audio_hex = parsed
            .pointer("/data/audio")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::Parse("minimax: music response without data.audio".into())
            })?;
        let bytes = hex::decode(audio_hex.trim())
            .map_err(|e| ProviderError::Parse(format!("minimax: audio hex decode: {e}")))?;
        if bytes.is_empty() {
            return Err(ProviderError::Parse("minimax: empty audio".into()));
        }
        Ok(bytes)
    }

    /// 提交任务：H3 → `POST /v2/video_generation`（multimodal content，支持
    /// 参考图 i2v）；其余模型 → `POST /v1/video_generation`（flat fields，
    /// 首参考图 → `first_frame_image`）。渠道 `param_override` 浅合并
    /// （callback_url 由节点 hook 链传入，优先于覆盖）。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        if request.prompt.trim().is_empty() {
            return Err(ProviderError::Config(
                "minimax: prompt must not be empty".into(),
            ));
        }
        let owned = ["resolution", "ratio", "duration"];
        let (path, mut body);
        if is_h3(model) {
            let duration = self.h3_duration(request)?;
            let resolution = self.h3_resolution(request)?;
            let refs: Vec<Value> = request
                .input_references
                .iter()
                .filter_map(|r| {
                    r.to_wire_string().map(|w| {
                        json!({
                            "type": "image_url",
                            "image_url": { "url": w },
                            "role": "first_frame",
                        })
                    })
                })
                .collect();
            let has_visual = !refs.is_empty();
            let ratio = self.h3_ratio(request, has_visual)?;
            let mut content = vec![json!({ "type": "text", "text": request.prompt })];
            content.extend(refs);
            body = json!({
                "model": model,
                "content": content,
                "resolution": resolution,
                "duration": duration,
                "ratio": ratio,
            });
            self.merge_overrides(&mut body, &owned);
            path = "/v2/video_generation";
        } else {
            body = json!({
                "model": model,
                "prompt": request.prompt,
                "duration": self.v1_duration(request),
                "resolution": self.v1_resolution(request, model),
            });
            self.merge_overrides(&mut body, &owned);
            if let Some(wire) = request
                .input_references
                .first()
                .and_then(|r| r.to_wire_string())
            {
                body["first_frame_image"] = Value::String(wire);
            }
            path = "/v1/video_generation";
        }
        if let Some(hook) = &request.callback_url {
            body["callback_url"] = Value::String(hook.clone());
        }
        let parsed = self.post_json(path, &body).await?;
        self.check_envelope(&parsed)?;
        let task_id = parsed
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(
                "minimax: submit response without task_id".into(),
            ));
        }
        Ok(VideoTask {
            id: task_id,
            status: self.status_from_wire(&parsed, is_h3(model)),
            progress: None,
            error: None,
        })
    }

    /// 轮询任务：H3 `GET /v2/query/video_generation/{id}`；
    /// V1 `GET /v1/query/video_generation?task_id=…`。
    async fn video_query(&self, task_id: &str, model: &str) -> Result<VideoTask, ProviderError> {
        let parsed = if is_h3(model) {
            self.get_json(&format!(
                "/v2/query/video_generation/{}",
                urlencoding::encode(task_id)
            ))
            .await?
        } else {
            self.get_json(&format!(
                "/v1/query/video_generation?task_id={}",
                urlencoding::encode(task_id)
            ))
            .await?
        };
        self.check_envelope(&parsed)?;
        let failed = self.status_from_wire(&parsed, is_h3(model)) == VideoStatus::Failed;
        let error = if failed {
            let h3_msg = self
                .h3_task(&parsed)
                .and_then(|t| t.pointer("/error/message"))
                .and_then(Value::as_str);
            Some(
                h3_msg
                    .or_else(|| {
                        parsed
                            .pointer("/base_resp/status_msg")
                            .and_then(Value::as_str)
                    })
                    .map(|m| self.redact(m))
                    .unwrap_or_else(|| "task failed".into()),
            )
        } else {
            None
        };
        Ok(VideoTask {
            id: task_id.to_string(),
            status: self.status_from_wire(&parsed, is_h3(model)),
            progress: None,
            error,
        })
    }

    /// 拉取成片：H3 = `task.content.url` 公网 CDN 直下；V1 = `file_id` →
    /// `GET /v1/files/download?file_id=…`（带鉴权）。
    async fn video_content(&self, task_id: &str, model: &str) -> Result<Vec<u8>, ProviderError> {
        let h3 = is_h3(model);
        let parsed = if h3 {
            self.get_json(&format!(
                "/v2/query/video_generation/{}",
                urlencoding::encode(task_id)
            ))
            .await?
        } else {
            self.get_json(&format!(
                "/v1/query/video_generation?task_id={}",
                urlencoding::encode(task_id)
            ))
            .await?
        };
        let url: String = if h3 {
            self.h3_task(&parsed)
                .and_then(|t| t.pointer("/content/url"))
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            parsed.get("file_id").and_then(Value::as_str).map(|fid| {
                format!(
                    "{}/v1/files/download?file_id={}",
                    self.base_url.trim_end_matches('/'),
                    urlencoding::encode(fid)
                )
            })
        }
        .ok_or_else(|| {
            ProviderError::Parse(format!("minimax: task {task_id} has no output url/file_id"))
        })?;
        let mut req = self.http.get(&url);
        if !h3 {
            // V1 file downloads are authed; H3 CDN URLs are pre-signed.
            req = self.apply_headers(req);
        }
        let resp = req
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: format!("minimax: download failed {status}"),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisfast_agent::provider::VideoInputRef;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(base_url: String) -> MiniMaxProvider {
        MiniMaxProvider::new(base_url, Some("mk-test".into()), None, None)
    }

    #[tokio::test]
    async fn h3_submit_posts_v2_content_array() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/video_generation"))
            .and(header("authorization", "Bearer mk-test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"task_id": "t-1", "status": "queued"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p
            .video_submit(
                &VideoRequest {
                    prompt: "夜景城市".into(),
                    seconds: Some("10".into()),
                    size: None,
                    input_references: Vec::new(),
                    callback_url: Some("https://me/hooks/cb1".into()),
                },
                "MiniMax-H3",
            )
            .await
            .unwrap();
        assert_eq!(task.id, "t-1");
        assert_eq!(task.status, VideoStatus::Queued);

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "MiniMax-H3");
        assert_eq!(body["duration"], 10);
        assert_eq!(body["resolution"], "768P");
        assert_eq!(body["ratio"], "16:9", "无视觉内容默认 16:9");
        assert_eq!(body["callback_url"], "https://me/hooks/cb1");
    }

    #[tokio::test]
    async fn h3_reference_images_switch_ratio_adaptive() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/video_generation"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"task_id": "t-2", "status": "queued"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &VideoRequest {
                prompt: "同款镜头".into(),
                seconds: None,
                size: Some("2k".into()),
                input_references: vec![VideoInputRef::from_url("https://cdn.example.com/a.png")],
                callback_url: None,
            },
            "MiniMax-H3",
        )
        .await
        .unwrap();

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["resolution"], "2K");
        assert_eq!(body["ratio"], "adaptive");
        assert_eq!(body["duration"], H3_DEFAULT_DURATION);
        let img = &body["content"][1];
        assert_eq!(img["type"], "image_url");
        assert_eq!(img["role"], "first_frame");
    }

    #[tokio::test]
    async fn h3_duration_out_of_range_is_config_error() {
        let p = MiniMaxProvider::new("http://127.0.0.1:1", Some("k".into()), None, None);
        let err = p
            .h3_duration(&VideoRequest {
                prompt: "x".into(),
                seconds: Some("30".into()),
                size: None,
                input_references: Vec::new(),
                callback_url: None,
            })
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn v1_submit_maps_first_frame_and_default_resolution() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/video_generation"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "task_id": "t-3", "status": "Queueing",
                "base_resp": {"status_code": 0, "status_msg": ""}
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p
            .video_submit(
                &VideoRequest {
                    prompt: "同款镜头".into(),
                    seconds: None,
                    size: None,
                    input_references: vec![VideoInputRef::from_url(
                        "https://cdn.example.com/a.png",
                    )],
                    callback_url: None,
                },
                "MiniMax-Hailuo-02",
            )
            .await
            .unwrap();
        assert_eq!(
            task.status,
            VideoStatus::InProgress,
            "Queueing → InProgress [照抄 plugin statuses]"
        );

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "MiniMax-Hailuo-02");
        assert_eq!(body["duration"], 6, "V1 默认 6s");
        assert_eq!(body["resolution"], "768P", "02 系默认 768P");
        assert_eq!(body["first_frame_image"], "https://cdn.example.com/a.png");
    }

    /// Combined-key mock: api_key + group_id.
    fn provider_with_group(base_url: String) -> MiniMaxProvider {
        MiniMaxProvider::new(base_url, Some("mk-test:12345".into()), None, None)
    }

    fn speech_hex_response() -> Value {
        // MiniMax t2a_v2 returns data.audio as HEX-encoded audio.
        json!({
            "data": {"audio": hex::encode(b"ID3fake-mp3")},
            "base_resp": {"status_code": 0, "status_msg": "success"}
        })
    }

    #[tokio::test]
    async fn speech_posts_t2a_v2_with_group_id_and_decodes_hex() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .and(query_param("GroupId", "12345"))
            .and(header("authorization", "Bearer mk-test"))
            .and(wiremock::matchers::body_partial_json(json!({
                "model": "speech-02-hd",
                "text": "你好旁白",
                "voice_setting": {"voice_id": "alloy"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(speech_hex_response()))
            .mount(&server)
            .await;

        let p = provider_with_group(server.uri());
        let bytes = p.speech("你好旁白", "alloy", "speech-02-hd").await.unwrap();
        assert_eq!(bytes, b"ID3fake-mp3");
    }

    #[tokio::test]
    async fn speech_without_group_id_is_config_error() {
        // 单段 key = 仅视频可用；TTS 需要 GroupId。
        let p = MiniMaxProvider::new(
            "http://127.0.0.1:1",
            Some("only-api-key".into()),
            None,
            None,
        );
        let err = p.speech("x", "alloy", "speech-02-hd").await.unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn video_with_combined_key_uses_api_key_part_only() {
        // 组合 key 下视频请求：Bearer 只带 api_key 段，URL 不带 GroupId
        // [照抄 plugin：视频端点与 Group 无关]。
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/video_generation"))
            .and(header("authorization", "Bearer mk-test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"task_id": "t-9", "status": "queued"})),
            )
            .mount(&server)
            .await;

        let p = provider_with_group(server.uri());
        let task = p
            .video_submit(
                &VideoRequest {
                    prompt: "x".into(),
                    seconds: None,
                    size: None,
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "MiniMax-H3",
            )
            .await
            .unwrap();
        assert_eq!(task.id, "t-9");
        let reqs = server.received_requests().await.unwrap();
        assert!(reqs[0].url.as_str().contains("/v2/video_generation"));
        assert!(!reqs[0].url.as_str().contains("GroupId"));
    }

    #[tokio::test]
    async fn chat_posts_to_chatcompletion_v2_and_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/text/chatcompletion_v2"))
            .and(header("authorization", "Bearer mk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmpl-1",
                "choices": [{
                    "finish_reason": "stop",
                    "message": {"role": "assistant", "content": "答案是 42"}
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5},
                "base_resp": {"status_code": 0, "status_msg": "success"}
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let request = raisfast_agent::provider::ChatRequest {
            messages: &[raisfast_agent::ChatMessage::user("1+1=?")],
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let resp = p.chat(&request, "MiniMax-M2").await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("答案是 42"));
        assert_eq!(resp.usage.unwrap().input_tokens, Some(10));
    }

    #[tokio::test]
    async fn chat_parses_tool_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/text/chatcompletion_v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call-1", "type": "function",
                            "function": {"name": "get_weather", "arguments": "{\"city\": \"上海\"}"}
                        }]
                    }
                }]
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let request = raisfast_agent::provider::ChatRequest {
            messages: &[raisfast_agent::ChatMessage::user("天气?")],
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let resp = p.chat(&request, "MiniMax-M2").await.unwrap();
        assert_eq!(resp.tool_calls[0].name, "get_weather");
        assert_eq!(resp.tool_calls[0].id, "call-1");
    }

    #[tokio::test]
    async fn chat_base_resp_reject_maps_to_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/text/chatcompletion_v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp": {"status_code": 1004, "status_msg": "invalid api key"}
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let request = raisfast_agent::provider::ChatRequest {
            messages: &[raisfast_agent::ChatMessage::user("hi")],
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let err = p.chat(&request, "MiniMax-M2").await.unwrap_err();
        match err {
            ProviderError::Http { status, body } => {
                assert_eq!(status, 500);
                assert!(body.contains("1004"), "{body}");
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn h3_query_success_and_content_url_download() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/query/video_generation/t-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "task": {"task_id": "t-1", "status": "succeeded",
                         "content": {"url": "http://127.0.0.1:1/out.mp4"}}
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p.video_query("t-1", "MiniMax-H3").await.unwrap();
        assert_eq!(task.status, VideoStatus::Completed);
        let err = p.video_content("t-1", "MiniMax-H3").await.unwrap_err();
        assert!(
            matches!(
                err,
                ProviderError::Http { .. } | ProviderError::Transport(_)
            ),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn v1_query_file_id_downloads_with_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/query/video_generation"))
            .and(query_param("task_id", "t-3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "task_id": "t-3", "status": "Success", "file_id": "fid-9",
                "base_resp": {"status_code": 0}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/files/download"))
            .and(header("authorization", "Bearer mk-test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4")
                    .set_body_bytes(b"MP4-fake".to_vec()),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let bytes = p.video_content("t-3", "MiniMax-Hailuo-02").await.unwrap();
        assert_eq!(&bytes[..4], b"MP4-");
    }
}

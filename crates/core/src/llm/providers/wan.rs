//! Alibaba Wanx (通义万相) provider for internal consumption: video surface
//! over DashScope's async `video-synthesis` protocol — `provider: "wan"`
//! channels route onto `LlmRouter::call().video_*`.
//!
//! Reference matrix:
//! - protocol constants (submit `POST {base}/api/v1/services/aigc/
//!   {video-generation|image2video}/video-synthesis` with
//!   `X-DashScope-Async: enable`, poll `GET {base}/api/v1/tasks/{id}`,
//!   envelope `{output:{task_id,task_status},request_id}`, error envelope
//!   `{code,message}`, status vocabulary PENDING/RUNNING/SUCCEEDED +
//!   terminal FAILED/CANCELED/UNKNOWN, result `output.video_url` ||
//!   `output.results.video_url`, per-model kind/resolution/duration matrix,
//!   LEGACY_SIZES pixel table, `wan2.1-` → `wanx2.1-` and `-YYYY-MM-DD`
//!   model-key normalization, hard validation of resolution/duration —
//!   invalid values error rather than silently downgrade): [照抄 new-api
//!   `plugins/tasks/alibaba/plugin.js` — vendored, symbol-level].
//! - auth: static DashScope API key, `Authorization: Bearer <key>`
//!   [照抄 new-api].
//! - provider-for-internal-consumption shape: [照抄本仓
//!   `providers/kling.rs` / `providers/seedance.rs`] — same constructor
//!   surface; non-video modalities keep trait defaults (`ProviderError::
//!   Config`) so the kernel skips the channel (failover, no failure report).
//! - input-reference mapping (0 refs → t2v body `input.prompt`; 1 →
//!   first frame; 2 → first+last frame; >2 → `reference_image` media on
//!   kind=all only): [自造-适配] — new-api 的入口是自家 requestBody 方言
//!   （img_url/first_frame_url/media 直传），本仓 `VideoRequest` 只有
//!   `input_references`，按张数映射到对应的上游字段组合。
//! - deviations from the reference (flagged, reversible):
//!   [自造-裁剪] `wan2.2-s2v`（口播，需要 audio_url）与 `template`/
//!   `reference_video`/音频类 media 不支持——本仓 `VideoRequest` 无音频
//!   输入面，speech 模型在 submit 时报 `Config`；[自造-裁剪] W×H size
//!   先查 LEGACY_SIZES/MODERN_SIZES 反表（照抄 videoSize），查不到时按
//!   最长边就近归档 resolution + 宽高比就近归档 ratio（vidu plugin 的
//!   normalize 思路），而非报错——渠道复用 OpenAI 风格 `1280x720` 输入；
//!   [自造-适配] unknown task status → `InProgress`（分布式 sweep 收敛，
//!   同 seedance 先例）；[自造-适配] `param_override` 浅合并进
//!   `parameters` 且合并后的 resolution/duration 仍过 profile 硬校验
//!   （防超预期费用，同 seedance `resolution()` 先例）。

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use crate::llm::relay::adaptor::shared_client;

/// Model family — decides the submit path and the input shape
/// [照抄 new-api WAN_MODELS `kind` 字段].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WanKind {
    /// Unified endpoint, prompt and/or `input.media` (wan3.0).
    All,
    /// Text-to-video only, `input.prompt` (wan2.7-t2v).
    T2v,
    /// Legacy t2v: pixel `parameters.size` instead of resolution+ratio.
    Size,
    /// Image-to-video via `input.img_url` (wan2.6/2.5/2.2/wanx2.1 i2v).
    Image,
    /// I2v via `input.media` first/last frame + driving audio (wan2.7-i2v).
    Media,
    /// First/last frame via `input.first_frame_url`/`last_frame_url`
    /// (wan2.2-kf2v).
    Frames,
    /// Speech-driven (wan2.2-s2v) — needs audio, unsupported here.
    Speech,
}

/// Per-model capability/pricing tier [照抄 new-api WAN_MODELS 表].
struct WanProfile {
    kind: WanKind,
    resolutions: &'static [&'static str],
    default_resolution: &'static str,
    /// Fixed duration enum (paid tier); None = integer 2..=max_duration
    /// (kind All additionally accepts -1 smart duration).
    durations: Option<&'static [i64]>,
    max_duration: i64,
}

/// Expands to the last repetition arm without building a temporary array
/// (static promotion fails through `.last()`).
macro_rules! last_arm {
    ($x:expr) => {
        $x
    };
    ($x:expr, $($rest:expr),+) => {
        last_arm!($($rest),+)
    };
}

macro_rules! wan_profile {
    ($kind:ident, [$($res:literal),+], $def:literal, $max:literal) => {
        WanProfile {
            kind: WanKind::$kind,
            resolutions: &[$($res),+],
            default_resolution: $def,
            durations: None,
            max_duration: $max,
        }
    };
    ($kind:ident, [$($res:literal),+], $def:literal, [$($dur:literal),+]) => {
        WanProfile {
            kind: WanKind::$kind,
            resolutions: &[$($res),+],
            default_resolution: $def,
            durations: Some(&[$($dur),+]),
            max_duration: last_arm!($($dur),+),
        }
    };
}

/// [照抄 new-api WAN_MODELS] — 付费规格决定项（分辨率×时长档位）原样保留。
fn profile(model: &str) -> Option<&'static WanProfile> {
    let key = model_key(model);
    let p: &'static WanProfile = match key.as_str() {
        "wan3.0-video" => &wan_profile!(All, ["480P", "720P", "1080P"], "1080P", 30),
        "wan3.0-video-prime" => &wan_profile!(All, ["480P", "720P", "1080P"], "1080P", 30),
        "wan2.7-t2v" => &wan_profile!(T2v, ["720P", "1080P"], "1080P", 15),
        "wan2.7-i2v" => &wan_profile!(Media, ["720P", "1080P"], "1080P", 15),
        "wan2.6-t2v" => &wan_profile!(Size, ["720P", "1080P"], "1080P", 15),
        "wan2.6-t2v-us" => &wan_profile!(Size, ["720P", "1080P"], "1080P", [5, 10, 15]),
        "wan2.6-i2v" => &wan_profile!(Image, ["720P", "1080P"], "1080P", 15),
        "wan2.6-i2v-flash" => &wan_profile!(Image, ["720P", "1080P"], "1080P", 15),
        "wan2.6-i2v-us" => &wan_profile!(Image, ["720P", "1080P"], "1080P", [5, 10, 15]),
        "wan2.5-t2v-preview" => &wan_profile!(Size, ["480P", "720P", "1080P"], "1080P", [5, 10]),
        "wan2.5-i2v-preview" => &wan_profile!(Image, ["480P", "720P", "1080P"], "1080P", [5, 10]),
        "wan2.2-t2v-plus" => &wan_profile!(Size, ["480P", "1080P"], "1080P", [5]),
        "wan2.2-i2v-flash" => &wan_profile!(Image, ["480P", "720P", "1080P"], "720P", [5]),
        "wan2.2-i2v-plus" => &wan_profile!(Image, ["480P", "1080P"], "1080P", [5]),
        "wan2.2-kf2v-flash" => &wan_profile!(Frames, ["480P", "720P", "1080P"], "720P", [5]),
        "wan2.2-s2v" => &wan_profile!(Speech, ["480P", "720P"], "480P", [20]),
        "wanx2.1-t2v-plus" => &wan_profile!(Size, ["720P"], "720P", [5]),
        "wanx2.1-t2v-turbo" => &wan_profile!(Size, ["480P", "720P"], "720P", [5]),
        "wanx2.1-i2v-plus" => &wan_profile!(Image, ["720P"], "720P", [5]),
        "wanx2.1-i2v-turbo" => &wan_profile!(Image, ["480P", "720P"], "720P", [3, 4, 5]),
        _ => return None,
    };
    Some(p)
}

/// [照抄 new-api `modelKey`] — 去掉 `-YYYY-MM-DD` 日期后缀，`wan2.1-`
/// 前缀归一到 `wanx2.1-`。
fn model_key(model: &str) -> String {
    let key = model.trim();
    // `YYYY-MM-DD` (10 chars, dashes at fixed offsets 4 and 7).
    let is_date = |s: &str| -> bool {
        let b = s.as_bytes();
        b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| c.is_ascii_digit() || (i == 4 || i == 7) && *c == b'-')
    };
    let mut base = key;
    if key.len() >= 11 {
        let (head, tail) = key.split_at(key.len() - 11);
        if tail.starts_with('-') && is_date(&tail[1..]) && !head.is_empty() {
            base = head;
        }
    }
    if let Some(rest) = base.strip_prefix("wan2.1-") {
        return format!("wanx2.1-{rest}");
    }
    base.to_string()
}

/// Pixel tables [照抄 new-api LEGACY_SIZES / MODERN_SIZES] — `kind=Size`
/// models take a `W*H` pixel string; the modern table only feeds reverse
/// lookup (a `1280x720` request maps back to 720P/16:9).
fn legacy_pixel_size(resolution: &str, ratio: &str) -> Option<&'static str> {
    Some(match (resolution, ratio) {
        ("720P", "16:9") => "1280*720",
        ("720P", "9:16") => "720*1280",
        ("720P", "1:1") => "960*960",
        ("720P", "4:3") => "1104*832",
        ("720P", "3:4") => "832*1104",
        ("1080P", "16:9") => "1920*1080",
        ("1080P", "9:16") => "1080*1920",
        ("1080P", "1:1") => "1440*1440",
        ("1080P", "4:3") => "1632*1248",
        ("1080P", "3:4") => "1248*1632",
        ("480P", "16:9") => "832*480",
        ("480P", "9:16") => "480*832",
        ("480P", "1:1") => "672*672",
        ("480P", "4:3") => "768*576",
        ("480P", "3:4") => "576*768",
        _ => return None,
    })
}

/// Nearest-tier mapping for an open `WxH`/`W*H` size [自造-适配] — longest
/// edge picks the resolution tier, aspect picks the ratio label (same
/// nearest-candidate approach as `providers::aspect_ratio_from_size`).
fn size_to_tiers(size: &str) -> Option<(String, String)> {
    let normalized = size.trim().replace('x', "*");
    let (w, h) = normalized.split_once('*')?;
    let w: f64 = w.trim().parse().ok()?;
    let h: f64 = h.trim().parse().ok()?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let max = w.max(h);
    let resolution = if max >= 1620.0 {
        "1080P"
    } else if max >= 1000.0 {
        "720P"
    } else {
        "480P"
    };
    let ratio = w / h;
    let candidates = [
        (16.0 / 9.0, "16:9"),
        (9.0 / 16.0, "9:16"),
        (1.0, "1:1"),
        (4.0 / 3.0, "4:3"),
        (3.0 / 4.0, "3:4"),
    ];
    let (_, label) = candidates
        .iter()
        .min_by(|a, b| (a.0 - ratio).abs().total_cmp(&(b.0 - ratio).abs()))?;
    Some((resolution.to_string(), (*label).to_string()))
}

/// `output.task_status` → our `VideoStatus` [照抄 new-api parseTaskResult；
/// unknown → InProgress 为本仓 sweep 适配].
fn status_from_wire(v: &Value) -> VideoStatus {
    match v
        .get("output")
        .and_then(|o| o.get("task_status"))
        .and_then(Value::as_str)
    {
        Some("PENDING") => VideoStatus::Queued,
        Some("RUNNING") => VideoStatus::InProgress,
        Some("SUCCEEDED") => VideoStatus::Completed,
        Some("FAILED") | Some("CANCELED") | Some("UNKNOWN") => VideoStatus::Failed,
        _ => VideoStatus::InProgress,
    }
}

/// First result video URL [照抄 new-api videoURL].
fn result_url(v: &Value) -> Option<String> {
    let output = v.get("output")?;
    output
        .get("video_url")
        .and_then(Value::as_str)
        .or_else(|| {
            output
                .get("results")
                .and_then(|r| r.get("video_url"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

/// Failure reason [照抄 new-api parseTaskResult reason 链].
fn failure_reason(v: &Value) -> Option<String> {
    if status_from_wire(v) != VideoStatus::Failed {
        return None;
    }
    let output = v.get("output");
    let reason = v
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            output.and_then(|o| {
                o.get("message").and_then(Value::as_str).map(|m| {
                    format!(
                        "task failed, code: {} , message: {m}",
                        o.get("code").and_then(Value::as_str).unwrap_or("")
                    )
                })
            })
        })
        .unwrap_or_else(|| "task failed".into());
    Some(reason)
}

pub struct WanProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

impl WanProvider {
    /// `base_url` is the DashScope root, e.g. `https://dashscope.aliyuncs.com`.
    pub fn new(
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

    fn apply_headers(
        &self,
        req: reqwest::RequestBuilder,
        async_header: bool,
    ) -> reqwest::RequestBuilder {
        let mut req = req.bearer_auth(self.api_key.clone().unwrap_or_default());
        if async_header {
            req = req.header("X-DashScope-Async", "enable");
        }
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

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, ProviderError> {
        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Self::unwrap_envelope(status, &text)
    }

    /// Unwrap DashScope's `{code, message, …}` envelope: non-2xx → `Http`
    /// with upstream status; 2xx + non-zero `code` → synthetic `Http 500`
    /// (transient → kernel failover) [照抄 kling envelope 先例 + new-api
    /// parseSubmitResponse `if (body.code) throw`].
    fn unwrap_envelope(status: u16, text: &str) -> Result<Value, ProviderError> {
        let parsed: Value =
            serde_json::from_str(text).map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        let message = parsed
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| text.lines().next().unwrap_or_default().to_string());
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: message,
            });
        }
        let code = parsed.get("code");
        // JS truthy semantics [照抄 new-api `if (body.code) throw`] —
        // DashScope 的 code 是字符串（"InvalidApiKey"），null/缺失/"0"/0
        // 之外都视为错误。
        let code_present = match code {
            None | Some(Value::Null) => false,
            Some(Value::String(s)) => !s.is_empty() && s != "0",
            Some(Value::Number(n)) => n.as_i64().unwrap_or(1) != 0,
            Some(_) => true,
        };
        if code_present {
            let code_str = match code {
                Some(Value::String(s)) => s.clone(),
                other => other.map(|v| v.to_string()).unwrap_or_default(),
            };
            return Err(ProviderError::Http {
                status: 500,
                body: format!("dashscope code {code_str}: {message}"),
            });
        }
        Ok(parsed)
    }

    /// Hard-validated resolution [照抄 new-api convert: `resolution must be
    /// one of …` — 无效值报错而非静默回退，防超预期费用].
    fn validate_resolution(
        resolution: &str,
        p: &WanProfile,
        model: &str,
    ) -> Result<(), ProviderError> {
        if p.resolutions.contains(&resolution) {
            return Ok(());
        }
        Err(ProviderError::Config(format!(
            "wan: {model} resolution must be one of {}",
            p.resolutions.join(", ")
        )))
    }

    /// Hard-validated duration [照抄 new-api convert duration 分支].
    fn validate_duration(duration: i64, p: &WanProfile, model: &str) -> Result<(), ProviderError> {
        if let Some(enum_) = p.durations {
            if enum_.contains(&duration) {
                return Ok(());
            }
            return Err(ProviderError::Config(format!(
                "wan: {model} duration must be one of {}",
                enum_
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let smart_ok = p.kind == WanKind::All && duration == -1;
        if smart_ok || (duration >= 2 && duration <= p.max_duration) {
            return Ok(());
        }
        Err(ProviderError::Config(format!(
            "wan: {model} duration must be {}an integer between 2 and {}",
            if p.kind == WanKind::All { "-1 or " } else { "" },
            p.max_duration
        )))
    }

    /// Build `{model, input, parameters}` [照抄 new-api convert — 见文件头
    /// reference matrix 的映射/裁剪标注].
    fn build_body(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<(Value, &'static WanProfile), ProviderError> {
        let Some(p) = profile(model) else {
            return Err(ProviderError::Config(format!(
                "wan: unsupported model {model:?}"
            )));
        };
        if p.kind == WanKind::Speech {
            return Err(ProviderError::Config(
                "wan: wan2.2-s2v needs an audio input surface this provider does not expose".into(),
            ));
        }

        let mut input = json!({});
        let refs: Vec<Option<String>> = request
            .input_references
            .iter()
            .map(|r| r.to_wire_string())
            .collect();
        if !request.prompt.trim().is_empty() {
            input["prompt"] = Value::String(request.prompt.trim().to_string());
        }
        match (p.kind, refs.as_slice()) {
            (WanKind::T2v | WanKind::Size, []) => {}
            (WanKind::All, []) => {}
            (WanKind::Image, [Some(first)]) => {
                input["img_url"] = Value::String(first.clone());
            }
            (WanKind::Media, [Some(first)]) => {
                input["media"] = json!([{ "type": "first_frame", "url": first }]);
            }
            (WanKind::Media, [Some(first), Some(last)]) => {
                input["media"] = json!([
                    { "type": "first_frame", "url": first },
                    { "type": "last_frame", "url": last },
                ]);
            }
            (WanKind::Frames, [Some(first)]) => {
                input["first_frame_url"] = Value::String(first.clone());
            }
            (WanKind::Frames, [Some(first), Some(last)]) => {
                input["first_frame_url"] = Value::String(first.clone());
                input["last_frame_url"] = Value::String(last.clone());
            }
            (WanKind::All, imgs) if imgs.len() > 2 => {
                let media: Vec<Value> = imgs
                    .iter()
                    .flatten()
                    .map(|u| json!({ "type": "reference_image", "url": u }))
                    .collect();
                input["media"] = Value::Array(media);
            }
            (kind, refs) => {
                return Err(ProviderError::Config(format!(
                    "wan: model {model} (kind {:?}) does not take {} reference image(s)",
                    kind,
                    refs.len()
                )));
            }
        }

        // parameters [照抄 new-api convert: prompt_extend 默认开，resolution
        // /duration/ratio/size 按 kind 组装]。
        let mut parameters = json!({ "prompt_extend": true });
        if let Some((resolution, ratio)) = request.size.as_deref().and_then(size_to_tiers) {
            match p.kind {
                WanKind::Size => {
                    // Legacy t2v 协议显像素串 [照抄 LEGACY_SIZES]。
                    let pixel = legacy_pixel_size(&resolution, &ratio).ok_or_else(|| {
                        ProviderError::Config(format!(
                            "wan: unsupported ratio for {resolution}: {ratio}"
                        ))
                    })?;
                    parameters["size"] = Value::String(pixel.to_string());
                }
                WanKind::T2v | WanKind::All => {
                    parameters["resolution"] = Value::String(resolution);
                    parameters["ratio"] = Value::String(ratio);
                }
                _ => {
                    // Image-derived aspect ratios are not configurable [照抄].
                    parameters["resolution"] = Value::String(resolution);
                }
            }
        } else {
            parameters["resolution"] = Value::String(p.default_resolution.to_string());
            if p.kind == WanKind::T2v || p.kind == WanKind::Size {
                parameters["ratio"] = Value::String("16:9".into());
            } else if p.kind == WanKind::All {
                parameters["ratio"] = Value::String("adaptive".into());
            }
        }
        // Duration: None = provider default 5 [照抄 `duration == null ? 5`].
        let duration = match request.seconds.as_deref() {
            None => 5,
            Some(s) => s.trim().parse::<i64>().map_err(|_| {
                ProviderError::Config(format!("wan: duration must be a number, got {s:?}"))
            })?,
        };
        parameters["duration"] = json!(duration);

        // Channel overrides shallow-merge into parameters (seed/prompt_extend/
        // watermark/audio/shot_type…), then the merged paid-tier knobs are
        // still hard-validated [自造-适配，见文件头].
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object)
            && let Some(target) = parameters.as_object_mut()
        {
            for (k, v) in map {
                target.insert(k.clone(), v.clone());
            }
        }
        if let Some(res) = parameters.get("resolution").and_then(Value::as_str) {
            Self::validate_resolution(res, p, model)?;
        }
        if let Some(dur) = parameters.get("duration").and_then(Value::as_i64) {
            Self::validate_duration(dur, p, model)?;
        }

        Ok((
            json!({ "model": model, "input": input, "parameters": parameters }),
            p,
        ))
    }
}

#[async_trait]
impl ModelProvider for WanProvider {
    fn name(&self) -> &str {
        "wan"
    }

    /// Required by the trait; Wan has no chat surface — return `Config` so
    /// the kernel skips (not fails) this channel.
    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Config(
            "provider wan does not support chat".into(),
        ))
    }

    /// 提交异步视频任务：t2v → `video-generation/video-synthesis`；i2v
    /// （image/frames）→ `image2video/video-synthesis`；media/all（wan2.7-i2v
    /// /wan3.0）→ `video-generation` + `input.media` [照抄 new-api
    /// buildSubmitRequest service 选择]。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let (body, p) = self.build_body(request, model)?;
        let service = if matches!(p.kind, WanKind::Image | WanKind::Frames) {
            "image2video"
        } else {
            "video-generation"
        };
        let url = format!(
            "{}/api/v1/services/aigc/{service}/video-synthesis",
            self.base_url.trim_end_matches('/')
        );
        let req = self.apply_headers(self.http.post(&url).json(&body), true);
        let parsed = self.send(req).await?;
        let task_id = parsed
            .get("output")
            .and_then(|o| o.get("task_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(
                "wan: envelope without output.task_id".into(),
            ));
        }
        Ok(VideoTask {
            id: task_id,
            status: status_from_wire(&parsed),
            progress: None,
            error: None,
        })
    }

    /// 轮询任务：`GET {base}/api/v1/tasks/{id}`。
    async fn video_query(&self, task_id: &str, _model: &str) -> Result<VideoTask, ProviderError> {
        let parsed = self.get_task(task_id).await?;
        Ok(VideoTask {
            id: task_id.to_string(),
            status: status_from_wire(&parsed),
            progress: None,
            error: failure_reason(&parsed),
        })
    }

    /// 拉取成片：查询拿 `output.video_url`（有效期限制，调用方应及时
    /// 转存）后下载字节。
    async fn video_content(&self, task_id: &str, _model: &str) -> Result<Vec<u8>, ProviderError> {
        let parsed = self.get_task(task_id).await?;
        let Some(url) = result_url(&parsed) else {
            return Err(ProviderError::Parse(format!(
                "wan: task {task_id} has no result video url"
            )));
        };
        let resp = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: format!("wan: download failed {status}"),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

impl WanProvider {
    async fn get_task(&self, task_id: &str) -> Result<Value, ProviderError> {
        let url = format!(
            "{}/api/v1/tasks/{task_id}",
            self.base_url.trim_end_matches('/')
        );
        let req = self.apply_headers(self.http.get(&url), false);
        self.send(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisfast_agent::provider::VideoInputRef;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(base_url: String) -> WanProvider {
        WanProvider::new(base_url, Some("ds-key".into()), None, None)
    }

    fn video_request(prompt: &str, refs: Vec<VideoInputRef>) -> VideoRequest {
        VideoRequest {
            prompt: prompt.into(),
            seconds: None,
            size: None,
            input_references: refs,
            callback_url: None,
        }
    }

    #[tokio::test]
    async fn t2v_submit_posts_dashscope_async_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/services/aigc/video-generation/video-synthesis",
            ))
            .and(header("x-dashscope-async", "enable"))
            .and(header("authorization", "Bearer ds-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "request_id": "r-1",
                "output": { "task_id": "wan-t-1", "task_status": "PENDING" }
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p
            .video_submit(
                &VideoRequest {
                    prompt: "夜景城市".into(),
                    seconds: Some("5".into()),
                    size: Some("1920x1080".into()),
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "wan2.7-t2v",
            )
            .await
            .unwrap();
        assert_eq!(task.id, "wan-t-1");
        assert_eq!(task.status, VideoStatus::Queued);

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "wan2.7-t2v");
        assert_eq!(body["input"]["prompt"], "夜景城市");
        assert_eq!(body["parameters"]["resolution"], "1080P");
        assert_eq!(body["parameters"]["ratio"], "16:9");
        assert_eq!(body["parameters"]["duration"], 5);
        assert_eq!(body["parameters"]["prompt_extend"], true);
    }

    #[tokio::test]
    async fn legacy_size_model_emits_pixel_size() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/services/aigc/video-generation/video-synthesis",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": { "task_id": "t", "task_status": "PENDING" }
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &VideoRequest {
                prompt: "x".into(),
                seconds: Some("5".into()),
                size: Some("1280x720".into()),
                input_references: Vec::new(),
                callback_url: None,
            },
            "wan2.6-t2v",
        )
        .await
        .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["parameters"]["size"], "1280*720");
        assert!(body["parameters"].get("resolution").is_none());
    }

    #[tokio::test]
    async fn i2v_model_routes_image2video_with_img_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/aigc/image2video/video-synthesis"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": { "task_id": "t", "task_status": "PENDING" }
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &video_request(
                "同款镜头",
                vec![VideoInputRef::from_url("https://cdn.example.com/a.png")],
            ),
            "wan2.6-i2v",
        )
        .await
        .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["input"]["img_url"], "https://cdn.example.com/a.png");
    }

    #[tokio::test]
    async fn wan27_i2v_media_first_last_frame() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/services/aigc/video-generation/video-synthesis",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": { "task_id": "t", "task_status": "PENDING" }
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &video_request(
                "首尾帧",
                vec![
                    VideoInputRef::from_url("https://cdn.example.com/first.png"),
                    VideoInputRef::from_url("https://cdn.example.com/last.png"),
                ],
            ),
            "wan2.7-i2v",
        )
        .await
        .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["input"]["media"][0]["type"], "first_frame");
        assert_eq!(body["input"]["media"][1]["type"], "last_frame");
    }

    #[tokio::test]
    async fn wan30_reference_images_and_smart_duration() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/services/aigc/video-generation/video-synthesis",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": { "task_id": "t", "task_status": "PENDING" }
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &VideoRequest {
                prompt: "参考生成".into(),
                seconds: Some("-1".into()),
                size: None,
                input_references: vec![
                    VideoInputRef::from_url("https://cdn.example.com/1.png"),
                    VideoInputRef::from_url("https://cdn.example.com/2.png"),
                    VideoInputRef::from_url("https://cdn.example.com/3.png"),
                ],
                callback_url: None,
            },
            "wan3.0-video",
        )
        .await
        .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["parameters"]["duration"], -1);
        assert_eq!(body["input"]["media"].as_array().unwrap().len(), 3);
        assert_eq!(body["input"]["media"][0]["type"], "reference_image");
    }

    #[tokio::test]
    async fn unsupported_model_is_config_error() {
        let p = provider("http://127.0.0.1:1".into());
        let err = p
            .video_submit(&video_request("x", Vec::new()), "wan9.9-t2v")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn invalid_duration_enum_rejected() {
        let p = provider("http://127.0.0.1:1".into());
        // wan2.5-t2v-preview durations = [5, 10]; 7 is off the paid tier.
        let err = p
            .video_submit(
                &VideoRequest {
                    prompt: "x".into(),
                    seconds: Some("7".into()),
                    size: None,
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "wan2.5-t2v-preview",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn smart_duration_rejected_on_non_wan30() {
        let p = provider("http://127.0.0.1:1".into());
        let err = p
            .video_submit(
                &VideoRequest {
                    prompt: "x".into(),
                    seconds: Some("-1".into()),
                    size: None,
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "wan2.7-t2v",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn s2v_speech_model_unsupported() {
        let p = provider("http://127.0.0.1:1".into());
        let err = p
            .video_submit(&video_request("x", Vec::new()), "wan2.2-s2v")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn query_maps_status_and_error_reason() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks/wan-t-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": { "task_id": "wan-t-1", "task_status": "FAILED", "code": "InvalidParameter", "message": "bad image" }
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p.video_query("wan-t-1", "wan2.7-t2v").await.unwrap();
        assert_eq!(task.status, VideoStatus::Failed);
        assert!(
            task.error
                .as_deref()
                .is_some_and(|e| e.contains("bad image")),
            "{:?}",
            task.error
        );
    }

    #[tokio::test]
    async fn query_success_reads_video_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks/wan-t-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {
                    "task_status": "SUCCEEDED",
                    "video_url": "https://oss.example.com/out.mp4"
                }
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let parsed = p.get_task("wan-t-2").await.unwrap();
        assert_eq!(
            result_url(&parsed).as_deref(),
            Some("https://oss.example.com/out.mp4")
        );
    }

    #[tokio::test]
    async fn envelope_error_over_http_200_maps_to_transient_500() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/services/aigc/video-generation/video-synthesis",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": "InvalidApiKey", "message": "Invalid API-key"
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let err = p
            .video_submit(&video_request("x", Vec::new()), "wan2.7-t2v")
            .await
            .unwrap_err();
        match err {
            ProviderError::Http { status, body } => {
                assert_eq!(status, 500);
                assert!(body.contains("InvalidApiKey"), "{body}");
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn param_override_merges_into_parameters_but_still_validated() {
        // 720P is legal for wan2.7-t2v.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/services/aigc/video-generation/video-synthesis",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": { "task_id": "t", "task_status": "PENDING" }
            })))
            .mount(&server)
            .await;

        let p = WanProvider::new(
            server.uri(),
            Some("k".into()),
            Some(json!({"resolution": "720P", "seed": 42})),
            None,
        );
        p.video_submit(&video_request("x", Vec::new()), "wan2.7-t2v")
            .await
            .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["parameters"]["resolution"], "720P");
        assert_eq!(body["parameters"]["seed"], 42);

        // 4K is not on any paid tier → Config, not silent downgrade.
        let bad = WanProvider::new(
            String::from("http://127.0.0.1:1"),
            Some("k".into()),
            Some(json!({"resolution": "4K"})),
            None,
        );
        let err = bad
            .video_submit(&video_request("x", Vec::new()), "wan2.7-t2v")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[test]
    fn model_key_normalization() {
        assert_eq!(model_key("wan2.7-t2v-2026-04-25"), "wan2.7-t2v");
        assert_eq!(model_key("wan2.1-t2v-plus"), "wanx2.1-t2v-plus");
        assert_eq!(model_key("wan3.0-video"), "wan3.0-video");
        assert!(profile("wan2.7-t2v-2026-04-25").is_some());
        assert!(profile("wan2.1-i2v-turbo").is_some());
        assert!(profile("nope").is_none());
    }
}

//! Vidu (生数) provider for internal consumption: video surface over
//! Vidu's `/ent/v2` submit/poll protocol — `provider: "vidu"` channels
//! route onto `LlmRouter::call().video_*`.
//!
//! Reference matrix:
//! - protocol constants (submit `POST {base}/ent/v2/{text2video|img2video|
//!   start-end2video|reference2video}`, poll `GET {base}/ent/v2/tasks/{id}/
//!   creations`, auth `Authorization: Token <key>` — **Token**, not Bearer —
//!   body `{model, prompt?, images?, duration, resolution,
//!   movement_amplitude:"auto"}`, status vocabulary created/queueing →
//!   queued, processing → in-progress, success/failed terminal, result
//!   `creations[0].url`, failure reason `err_code`, model×duration×
//!   resolution combo validation, per-model duration/resolution defaults):
//!   [照抄 new-api `plugins/tasks/vidu/plugin.js` — vendored, symbol-level].
//! - provider-for-internal-consumption shape: [照抄本仓
//!   `providers/kling.rs` / `providers/seedance.rs`] — same constructor
//!   surface; non-video modalities keep trait defaults (`ProviderError::
//!   Config`) so the kernel skips the channel (failover, no failure report).
//! - input-reference mapping (0 refs → text2video; 1 → img2video; 2 →
//!   start-end2video; >2 → reference2video with the model forced to
//!   `viduq2`): [照抄 new-api actionFor/pathFor + buildSubmitRequest 的
//!   `reference_to_video` 模型强制] — 以参考图张数驱动 action 选择。
//! - deviations from the reference (flagged, reversible):
//!   [自造-裁剪] `metadata` 透传/bgm/remix 等入口方言不搬——本仓
//!   `VideoRequest` 只有 prompt/seconds/size/refs；[自造-适配] unknown
//!   task state → `InProgress`（分布式 sweep 收敛，同 seedance 先例）；
//!   [自造-适配] `callback_url` 不映射（Vidu 协议无回调字段，poll-only）。
//! - paid-task discipline [照抄 seedance/MPT 先例]: 组合校验失败
//!   （时长×分辨率×模型非法组合）在 submit 前报 `Config`，防超预期费用。

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use crate::llm::relay::adaptor::shared_client;

/// [照抄 new-api RESOLUTIONS] — 付费档位。
const RESOLUTIONS: [&str; 4] = ["360p", "540p", "720p", "1080p"];

/// [照抄 new-api isQ2Model] — `viduq2*` 前缀。
fn is_q2_model(model: &str) -> bool {
    model.starts_with("viduq2")
}

/// [照抄 new-api defaultDuration]。
fn default_duration(model: &str) -> i64 {
    if model == "vidu2.0" { 4 } else { 5 }
}

/// [照抄 new-api defaultResolution]。
fn default_resolution(model: &str) -> &'static str {
    if is_q2_model(model) {
        "720p"
    } else if model == "vidu2.0" {
        "360p"
    } else {
        "1080p"
    }
}

/// [照抄 new-api normalizeResolution] — viduq1 锁 1080p；直接档位；
/// `WxH`/`W*x` 按最长边归档（1920→1080p、1280→720p、960→540p、其余
/// 360p）；否则模型默认。
fn normalize_resolution(value: &str, model: &str) -> String {
    if model == "viduq1" {
        return "1080p".into();
    }
    let raw = value.trim().to_lowercase();
    if RESOLUTIONS.contains(&raw.as_str()) {
        return raw;
    }
    let normalized = raw.replace('*', "x");
    let parts = normalized.split('x').collect::<Vec<_>>();
    if parts.len() == 2
        && let (Ok(w), Ok(h)) = (
            parts[0].trim().parse::<f64>(),
            parts[1].trim().parse::<f64>(),
        )
        && w > 0.0
        && h > 0.0
    {
        return match w.max(h) {
            max if max >= 1920.0 => "1080p",
            max if max >= 1280.0 => "720p",
            max if max >= 960.0 => "540p",
            _ => "360p",
        }
        .into();
    }
    default_resolution(model).into()
}

/// [照抄 new-api validateViduCombo] — 模型×时长×分辨率合法组合硬校验。
fn validate_combo(
    model: &str,
    duration: i64,
    resolution: &str,
    has_images: bool,
) -> Result<(), ProviderError> {
    let msg = |m: String| ProviderError::Config(format!("vidu: {m}"));
    if model == "vidu2.0" && !has_images {
        return Err(msg("vidu2.0 does not support text-to-video".into()));
    }
    if model == "viduq1" {
        if duration != 5 {
            return Err(msg("viduq1 duration must be 5".into()));
        }
        return Ok(());
    }
    if model == "vidu2.0" {
        if duration == 4 {
            if !["360p", "720p", "1080p"].contains(&resolution) {
                return Err(msg(
                    "vidu2.0 duration 4 only allows resolution 360p, 720p, or 1080p".into(),
                ));
            }
            return Ok(());
        }
        if duration == 8 {
            if resolution != "720p" {
                return Err(msg("vidu2.0 duration 8 only allows resolution 720p".into()));
            }
            return Ok(());
        }
        return Err(msg("vidu2.0 duration must be 4 or 8".into()));
    }
    if is_q2_model(model) {
        if !(1..=10).contains(&duration) {
            return Err(msg("viduq2 duration must be between 1 and 10".into()));
        }
        return Ok(());
    }
    // vidu1.5 等：1..=3600 [照抄 `seconds must be between 1 and 3600`]。
    if !(1..=3600).contains(&duration) {
        return Err(msg("seconds must be between 1 and 3600".into()));
    }
    Ok(())
}

/// [照抄 new-api pathFor]。
fn path_for_action(refs: usize) -> &'static str {
    match refs {
        0 => "/ent/v2/text2video",
        1 => "/ent/v2/img2video",
        2 => "/ent/v2/start-end2video",
        _ => "/ent/v2/reference2video",
    }
}

/// `state` → our `VideoStatus` [照抄 new-api parseTaskResult statuses；
/// unknown → InProgress 为本仓 sweep 适配].
fn status_from_wire(state: &str) -> VideoStatus {
    match state {
        "created" | "queueing" => VideoStatus::Queued,
        "processing" => VideoStatus::InProgress,
        "success" => VideoStatus::Completed,
        "failed" => VideoStatus::Failed,
        _ => VideoStatus::InProgress,
    }
}

/// First result URL [照抄 new-api `creations[0].url`].
fn result_url(v: &Value) -> Option<String> {
    v.get("creations")
        .and_then(Value::as_array)
        .and_then(|cs| cs.first())
        .and_then(|c| c.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub struct ViduProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

impl ViduProvider {
    /// `base_url` is the Vidu root (no `/ent`), e.g. `https://api.vidu.cn`
    /// or the overseas `https://api.vidu.com`.
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

    /// `Authorization: Token <key>` [照抄 new-api — Token scheme, not
    /// Bearer].
    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut req = req.header(
            "Authorization",
            format!("Token {}", self.api_key.clone().unwrap_or_default()),
        );
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
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        if !(200..300).contains(&status) {
            let body = parsed
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| text.lines().next().unwrap_or_default().to_string());
            return Err(ProviderError::Http { status, body });
        }
        Ok(parsed)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let req = self.apply_headers(self.http.post(&url).json(body));
        self.send(req).await
    }

    async fn get_task(&self, task_id: &str) -> Result<Value, ProviderError> {
        let url = format!(
            "{}/ent/v2/tasks/{task_id}/creations",
            self.base_url.trim_end_matches('/')
        );
        let req = self.apply_headers(self.http.get(&url));
        self.send(req).await
    }

    /// Build the submit body [照抄 new-api buildSubmitRequest/outbound*].
    fn build_body(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<(Value, String), ProviderError> {
        // reference2video 强制 viduq2 [照抄 new-api buildSubmitRequest
        // `reference_to_video` 分支]。
        let model = if request.input_references.len() > 2 && model.contains("viduq2") {
            "viduq2"
        } else if request.input_references.len() > 2 {
            return Err(ProviderError::Config(
                "vidu: reference-to-video (>2 images) requires a viduq2 model".into(),
            ));
        } else {
            model
        };

        let images: Vec<String> = request
            .input_references
            .iter()
            .filter_map(|r| r.to_wire_string())
            .collect();
        let duration = request
            .seconds
            .as_deref()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or_else(|| default_duration(model));
        let resolution = request
            .size
            .as_deref()
            .map(|s| normalize_resolution(s, model))
            .unwrap_or_else(|| default_resolution(model).into());
        validate_combo(model, duration, &resolution, !images.is_empty())?;

        let mut body = json!({
            "model": model,
            "duration": duration,
            "resolution": resolution,
            "movement_amplitude": "auto",
        });
        // 有图时 prompt 可空（可省略字段）[照抄 `if (!body.prompt) delete`]。
        if !request.prompt.trim().is_empty() {
            body["prompt"] = Value::String(request.prompt.trim().to_string());
        }
        if !images.is_empty() {
            body["images"] = json!(images);
        }
        // Channel overrides shallow-merge last (style/seed/bgm…), same
        // precedence as kling's merge_overrides.
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object)
            && let Some(target) = body.as_object_mut()
        {
            for (k, v) in map {
                target.insert(k.clone(), v.clone());
            }
        }
        Ok((body, model.to_string()))
    }
}

#[async_trait]
impl ModelProvider for ViduProvider {
    fn name(&self) -> &str {
        "vidu"
    }

    /// Required by the trait; Vidu has no chat surface — return `Config` so
    /// the kernel skips (not fails) this channel.
    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Config(
            "provider vidu does not support chat".into(),
        ))
    }

    /// 提交异步视频任务：按参考图张数选 action 端点（0→text2video、
    /// 1→img2video、2→start-end2video、>2→reference2video）。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let (body, _model) = self.build_body(request, model)?;
        let path = path_for_action(
            request
                .input_references
                .iter()
                .filter_map(|r| r.to_wire_string())
                .count(),
        );
        let parsed = self.post(path, &body).await?;
        // [照抄 parseSubmitResponse: state=failed 直接失败；缺 task_id 报错]。
        let state = parsed.get("state").and_then(Value::as_str).unwrap_or("");
        if state == "failed" {
            return Err(ProviderError::Http {
                status: 500,
                body: format!(
                    "vidu: task failed: {}",
                    parsed.get("err_code").and_then(Value::as_str).unwrap_or("")
                ),
            });
        }
        let task_id = parsed
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(
                "vidu: envelope without task_id".into(),
            ));
        }
        Ok(VideoTask {
            id: task_id,
            status: status_from_wire(state),
            progress: None,
            error: None,
        })
    }

    /// 轮询任务：`GET {base}/ent/v2/tasks/{id}/creations`。
    async fn video_query(&self, task_id: &str, _model: &str) -> Result<VideoTask, ProviderError> {
        let parsed = self.get_task(task_id).await?;
        let state = parsed.get("state").and_then(Value::as_str).unwrap_or("");
        let status = status_from_wire(state);
        let error = (status == VideoStatus::Failed)
            .then(|| {
                parsed
                    .get("err_code")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .flatten();
        Ok(VideoTask {
            id: task_id.to_string(),
            status,
            progress: None,
            error,
        })
    }

    /// 拉取成片：查询拿 `creations[0].url`（有效期限制，调用方应及时
    /// 转存）后下载字节。
    async fn video_content(&self, task_id: &str, _model: &str) -> Result<Vec<u8>, ProviderError> {
        let parsed = self.get_task(task_id).await?;
        let Some(url) = result_url(&parsed) else {
            return Err(ProviderError::Parse(format!(
                "vidu: task {task_id} has no result video url"
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
                body: format!("vidu: download failed {status}"),
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
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(base_url: String) -> ViduProvider {
        ViduProvider::new(base_url, Some("vu-key".into()), None, None)
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
    async fn t2v_submit_posts_token_auth_and_defaults() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ent/v2/text2video"))
            .and(header("authorization", "Token vu-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"task_id": "vu-1", "state": "queueing"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p
            .video_submit(&video_request("夜景城市", Vec::new()), "viduq1")
            .await
            .unwrap();
        assert_eq!(task.id, "vu-1");
        assert_eq!(task.status, VideoStatus::Queued);

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "viduq1");
        assert_eq!(body["prompt"], "夜景城市");
        assert_eq!(body["duration"], 5, "viduq1 default duration");
        assert_eq!(body["resolution"], "1080p", "viduq1 locked resolution");
        assert_eq!(body["movement_amplitude"], "auto");
        assert!(body.get("images").is_none());
    }

    #[tokio::test]
    async fn i2v_submit_routes_img2video_with_images() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ent/v2/img2video"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"task_id": "vu-2", "state": "processing"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p
            .video_submit(
                &video_request(
                    "同款镜头",
                    vec![VideoInputRef::from_url("https://cdn.example.com/a.png")],
                ),
                "viduq1",
            )
            .await
            .unwrap();
        assert_eq!(task.status, VideoStatus::InProgress);
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["images"][0], "https://cdn.example.com/a.png");
    }

    #[tokio::test]
    async fn two_refs_route_start_end_and_size_maps_resolution() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ent/v2/start-end2video"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"task_id": "vu-3", "state": "created"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &VideoRequest {
                prompt: "首尾帧".into(),
                seconds: Some("5".into()),
                size: Some("1920x1080".into()),
                input_references: vec![
                    VideoInputRef::from_url("https://cdn.example.com/first.png"),
                    VideoInputRef::from_url("https://cdn.example.com/last.png"),
                ],
                callback_url: None,
            },
            "viduq1",
        )
        .await
        .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["images"].as_array().unwrap().len(), 2);
        assert_eq!(body["resolution"], "1080p", "1920x1080 → 1080p");
    }

    #[tokio::test]
    async fn many_refs_force_viduq2_or_reject() {
        let p = provider("http://127.0.0.1:1".into());
        let refs = vec![
            VideoInputRef::from_url("https://cdn.example.com/1.png"),
            VideoInputRef::from_url("https://cdn.example.com/2.png"),
            VideoInputRef::from_url("https://cdn.example.com/3.png"),
        ];
        // vidu1.5 + 3 refs → Config.
        let err = p
            .video_submit(&video_request("参考", refs.clone()), "vidu1.5")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");

        // viduq2 + 3 refs → model forced to viduq2 (reference2video path).
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ent/v2/reference2video"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"task_id": "vu-4", "state": "queueing"})),
            )
            .mount(&server)
            .await;
        let p2 = provider(server.uri());
        p2.video_submit(&video_request("参考", refs), "viduq2")
            .await
            .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "viduq2");
    }

    #[tokio::test]
    async fn vidu20_combo_validation() {
        let p = provider("http://127.0.0.1:1".into());
        let refs = vec![VideoInputRef::from_url("https://cdn.example.com/a.png")];
        // vidu2.0 t2v 不支持 [照抄].
        let err = p
            .video_submit(&video_request("x", Vec::new()), "vidu2.0")
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
        // vidu2.0 + 8s + 非 720p → Config.
        let err = p
            .video_submit(
                &VideoRequest {
                    prompt: "x".into(),
                    seconds: Some("8".into()),
                    size: Some("1920x1080".into()),
                    input_references: refs,
                    callback_url: None,
                },
                "vidu2.0",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn submit_response_failed_state_is_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ent/v2/text2video"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "task_id": "vu-x", "state": "failed", "err_code": "CONTENT_RISK"
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let err = p
            .video_submit(&video_request("x", Vec::new()), "viduq1")
            .await
            .unwrap_err();
        match err {
            ProviderError::Http { status, body } => {
                assert_eq!(status, 500);
                assert!(body.contains("CONTENT_RISK"), "{body}");
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn query_maps_creations_and_err_code() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ent/v2/tasks/vu-9/creations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "state": "success",
                "creations": [{ "id": "c1", "url": "https://cdn.vidu.cn/out.mp4" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ent/v2/tasks/vu-f/creations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "state": "failed", "err_code": "QUOTA_EXCEEDED"
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p.video_query("vu-9", "viduq1").await.unwrap();
        assert_eq!(task.status, VideoStatus::Completed);
        let failed = p.video_query("vu-f", "viduq1").await.unwrap();
        assert_eq!(failed.status, VideoStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("QUOTA_EXCEEDED"));
    }

    #[test]
    fn resolution_normalization() {
        assert_eq!(normalize_resolution("720P", "viduq1"), "1080p");
        assert_eq!(normalize_resolution("1080p", "viduq2"), "1080p");
        assert_eq!(normalize_resolution("1280x720", "viduq2"), "720p");
        assert_eq!(normalize_resolution("960*960", "viduq2"), "540p");
        assert_eq!(normalize_resolution("junk", "viduq2"), "720p", "q2 default");
        assert_eq!(normalize_resolution("", "vidu2.0"), "360p", "2.0 default");
    }
}

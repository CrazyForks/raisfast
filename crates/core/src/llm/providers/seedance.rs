//! Seedance (火山方舟/即梦) provider for internal consumption: video surface
//! over Ark's `contents/generations/tasks` protocol — `provider: "seedance"`
//! channels route onto `LlmRouter::call().video_*`.
//!
//! Reference matrix:
//! - protocol constants (submit `POST {base}/contents/generations/tasks`,
//!   poll `GET …/{id}`, body `{model, content:[{type:text,…}], ratio,
//!   resolution, watermark}`, duration INT clamped 2..12, resolution enum
//!   {480p,720p,1080p} default **1080p** with hard validation, status
//!   vocabulary queued/running/succeeded + terminal
//!   failed/cancelled/canceled/expired, empty-prompt guard, key redaction in
//!   error bodies): [照抄 MPT `app/services/volcengine_seedance.py` —
//!   vendored, symbol-level].
//! - auth: static Ark API key, `Authorization: Bearer <key>` [照抄 MPT].
//! - image-to-video: `content` array gains an
//!   `{type:image_url, image_url:{url}, role:first_frame}` entry [参考 Ark
//!   公开文档——MPT 参考实现仅覆盖文生视频，i2v 为文档级扩展].
//! - deviations from the reference (flagged, reversible):
//!   [自造-适配] unknown task status → `InProgress`（继续轮询到 deadline），
//!   而非参考的 UnconfirmedTaskError 立即失败——本仓轮询是分布式 sweep，
//!   无本地等待循环，deadline 门收敛等价；[自造-适配] 轮询瞬时错误容忍由
//!   flows poller「跳过下轮再试」承担（等价参考的 MAX_POLL_RETRIES 退避）。
//! - paid-task discipline [照抄 MPT]: submit 无响应/5xx 视为「远端可能已建
//!   任务」，不自动重发（内核 failover 边界由 SideEffectGuard/重试条件钉死）。

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use crate::llm::relay::adaptor::shared_client;

/// [照抄 MPT DEFAULT_RESOLUTION = "1080p"] — 分辨率直接决定付费规格。
const DEFAULT_RESOLUTION: &str = "1080p";
const SUPPORTED_RESOLUTIONS: [&str; 3] = ["480p", "720p", "1080p"];
/// [照抄 MPT DEFAULT_MIN/MAX_DURATION_SECONDS = 2..12]。
const MIN_DURATION_SECS: i64 = 2;
const MAX_DURATION_SECS: i64 = 12;

pub struct SeedanceProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

impl SeedanceProvider {
    /// `base_url` is the Ark root including the version segment, e.g.
    /// `https://ark.cn-beijing.volces.com/api/v3`.
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

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut req = req.bearer_auth(self.api_key.clone().unwrap_or_default());
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

    /// Resolution from channel `param_override` — hard-validated against the
    /// enum [照抄 MPT `_resolution`: 无效值报错而非静默回退，防超预期费用]。
    fn resolution(&self) -> Result<&'static str, ProviderError> {
        let configured = self
            .param_override
            .as_ref()
            .and_then(|o| o.get("resolution"))
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_lowercase);
        match configured.as_deref() {
            None => Ok(DEFAULT_RESOLUTION),
            Some(v) => SUPPORTED_RESOLUTIONS
                .iter()
                .find(|s| **s == v)
                .copied()
                .ok_or_else(|| {
                    ProviderError::Config(format!(
                        "unsupported seedance resolution {v:?}; expected one of: {}",
                        SUPPORTED_RESOLUTIONS.join(", ")
                    ))
                }),
        }
    }

    /// Duration INT clamped to the model range [照抄 MPT `_duration_bounds`
    /// 2..12 + 提交日志说明收敛]。
    fn duration_seconds(request: &VideoRequest) -> i64 {
        let requested = request
            .seconds
            .as_deref()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(MIN_DURATION_SECS);
        requested.clamp(MIN_DURATION_SECS, MAX_DURATION_SECS)
    }

    fn build_body(&self, request: &VideoRequest, model: &str) -> Result<Value, ProviderError> {
        // 空提示词可能来自上游模板解析异常——付费源不能提交空任务
        // [照抄 MPT generate_videos 空词守卫]。
        if request.prompt.trim().is_empty() {
            return Err(ProviderError::Config(
                "seedance: prompt must not be empty".into(),
            ));
        }
        let mut content = vec![json!({ "type": "text", "text": request.prompt })];
        if let Some(wire) = request
            .input_references
            .first()
            .and_then(|r| r.to_wire_string())
        {
            // Image-to-video: first reference anchors the first frame
            // [参考 Ark 公开文档——MPT 参考实现仅覆盖文生视频].
            content.push(json!({
                "type": "image_url",
                "image_url": { "url": wire },
                "role": "first_frame",
            }));
        }
        let mut body = json!({
            "model": model,
            "content": content,
            // ratio 取参考的画布枚举；size 缺省 = 9:16（MPT 默认竖屏）。
            "ratio": super::aspect_ratio_from_size(request.size.as_deref())
                .unwrap_or_else(|| "9:16".to_string()),
            "duration": Self::duration_seconds(request),
            "resolution": self.resolution()?,
            "watermark": false,
        });
        // 其余渠道覆盖（cfg_scale/camera_fixed/callback_url…）浅合并。
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object) {
            for (k, v) in map {
                if k == "resolution" {
                    continue; // already validated + applied above
                }
                body[k.clone()] = v.clone();
            }
        }
        Ok(body)
    }

    /// 错误体脱敏 [照抄 MPT `_redact_secret`]：上游 key 不进日志/错误消息。
    fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some(key) = &self.api_key
            && !key.is_empty()
        {
            out = out.replace(key.as_str(), "***");
        }
        out.chars().take(500).collect()
    }

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

    async fn get_task(&self, task_id: &str) -> Result<Value, ProviderError> {
        let url = format!(
            "{}/contents/generations/tasks/{}",
            self.base_url.trim_end_matches('/'),
            task_id
        );
        self.send(self.http.get(&url)).await
    }

    /// Ark status vocabulary → our `VideoStatus`. Unknown status keeps
    /// polling (deadline gate converges) [自造-适配，见模块头].
    fn status_from_wire(v: &Value) -> VideoStatus {
        match v.get("status").and_then(Value::as_str) {
            Some("succeeded") => VideoStatus::Completed,
            Some("failed") | Some("cancelled") | Some("canceled") | Some("expired") => {
                VideoStatus::Failed
            }
            Some("queued") => VideoStatus::Queued,
            _ => VideoStatus::InProgress,
        }
    }

    fn result_url(v: &Value) -> Option<String> {
        v.get("content")
            .and_then(|c| c.get("video_url"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

#[async_trait]
impl ModelProvider for SeedanceProvider {
    fn name(&self) -> &str {
        "seedance"
    }

    /// Required by the trait; this provider is video-only (Ark chat goes
    /// through `provider: "openai"` channels) — `Config` so the kernel skips.
    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Config(
            "provider seedance does not support chat (use an openai channel for Ark chat)".into(),
        ))
    }

    /// 提交任务：`POST {base}/contents/generations/tasks`。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let body = self.build_body(request, model)?;
        let url = format!(
            "{}/contents/generations/tasks",
            self.base_url.trim_end_matches('/')
        );
        let parsed = self.send(self.http.post(&url).json(&body)).await?;
        let task_id = parsed
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(
                "seedance: task response without id".into(),
            ));
        }
        Ok(VideoTask {
            id: task_id,
            status: Self::status_from_wire(&parsed),
            progress: None,
            error: None,
        })
    }

    /// 轮询任务：`GET {base}/contents/generations/tasks/{id}`。
    async fn video_query(&self, task_id: &str, _model: &str) -> Result<VideoTask, ProviderError> {
        let parsed = self.get_task(task_id).await?;
        Ok(VideoTask {
            id: task_id.to_string(),
            status: Self::status_from_wire(&parsed),
            progress: None,
            error: parsed
                .get("error")
                .and_then(Value::as_str)
                .map(|e| self.redact(e)),
        })
    }

    /// 拉取成片：查询拿 `content.video_url`（CDN 链接有有效期）后下载字节。
    async fn video_content(&self, task_id: &str, _model: &str) -> Result<Vec<u8>, ProviderError> {
        let parsed = self.get_task(task_id).await?;
        let Some(url) = Self::result_url(&parsed) else {
            return Err(ProviderError::Parse(format!(
                "seedance: task {task_id} has no video_url"
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
                body: format!("seedance: download failed {status}"),
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
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(base_url: String) -> SeedanceProvider {
        SeedanceProvider::new(base_url, Some("ark-key".into()), None, None)
    }

    #[tokio::test]
    async fn submit_posts_content_array_with_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/contents/generations/tasks"))
            .and(header("authorization", "Bearer ark-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": "cgt-1", "status": "queued"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p
            .video_submit(
                &VideoRequest {
                    prompt: "夜景城市".into(),
                    seconds: Some("5".into()),
                    size: Some("1080x1920".into()),
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "doubao-seedance-1-0-pro",
            )
            .await
            .unwrap();
        assert_eq!(task.id, "cgt-1");
        assert_eq!(task.status, VideoStatus::Queued);

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "doubao-seedance-1-0-pro");
        assert_eq!(body["content"][0]["type"], "text");
        assert_eq!(body["ratio"], "9:16");
        // duration 是 INT 且被钳制到 2..12 [照抄 MPT]。
        assert_eq!(body["duration"], 5);
        assert_eq!(body["resolution"], "1080p");
        assert_eq!(body["watermark"], false);
    }

    #[tokio::test]
    async fn duration_clamped_to_model_range() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/contents/generations/tasks"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": "cgt-9", "status": "queued"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &VideoRequest {
                prompt: "x".into(),
                seconds: Some("60".into()),
                size: None,
                input_references: Vec::new(),
                callback_url: None,
            },
            "m",
        )
        .await
        .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["duration"], 12, "clamp to MAX");
    }

    #[tokio::test]
    async fn empty_prompt_rejected_before_submit() {
        let server = MockServer::start().await;
        // No mock mounted — any request would fail the test via connection error.
        let p = provider(server.uri());
        let err = p
            .video_submit(
                &VideoRequest {
                    prompt: "  ".into(),
                    seconds: None,
                    size: None,
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "m",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn invalid_resolution_is_config_error_not_silent() {
        let p = SeedanceProvider::new(
            "http://127.0.0.1:1",
            Some("k".into()),
            Some(json!({"resolution": "4K"})),
            None,
        );
        let err = p
            .video_submit(
                &VideoRequest {
                    prompt: "x".into(),
                    seconds: None,
                    size: None,
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "m",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[tokio::test]
    async fn reference_image_becomes_first_frame_entry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/contents/generations/tasks"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": "cgt-2", "status": "queued"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &VideoRequest {
                prompt: "同款镜头".into(),
                seconds: None,
                size: None,
                input_references: vec![VideoInputRef::from_url("https://cdn.example.com/a.png")],
                callback_url: None,
            },
            "seedance-lite",
        )
        .await
        .unwrap();

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        let img = &body["content"][1];
        assert_eq!(img["type"], "image_url");
        assert_eq!(img["role"], "first_frame");
        assert_eq!(img["image_url"]["url"], "https://cdn.example.com/a.png");
    }

    #[tokio::test]
    async fn query_maps_succeed_and_extracts_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/contents/generations/tasks/cgt-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cgt-1", "status": "succeeded",
                "content": {"video_url": "https://cdn.volces.com/out.mp4"}
            })))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p.video_query("cgt-1", "m").await.unwrap();
        assert_eq!(task.status, VideoStatus::Completed);
        // content path: video_content re-queries then downloads — wiring
        // covered by the refused download here (no live CDN in tests).
        let err = p.video_content("cgt-1", "m").await.unwrap_err();
        assert!(
            matches!(
                err,
                ProviderError::Http { .. } | ProviderError::Transport(_)
            ),
            "{err:?}"
        );
    }
}

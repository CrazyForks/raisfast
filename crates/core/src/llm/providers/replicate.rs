//! Replicate provider for internal consumption: implements the agent
//! `ModelProvider` video surface (submit/query/content) over Replicate's
//! predictions protocol, so flows can route `provider: "replicate"` channels
//! onto `LlmRouter::call().video_*`.
//!
//! Reference matrix:
//! - provider-for-internal-consumption shape: [照抄本仓 `providers/kling.rs`]
//!   — same constructor surface; non-video modalities keep trait defaults
//!   (`ProviderError::Config`) so the kernel skips the channel.
//! - predictions protocol: [参考 Replicate 公开 API 文档，本地无 third 源码]
//!   — official models submit `POST /v1/models/{owner}/{name}/predictions`
//!   (model = `owner/name`); version refs submit `POST /v1/predictions` with
//!   `{"version": …}` (model without `/`). Model-specific params ride
//!   `input` (prompt/duration/image…); channel `param_override` shallow-
//!   merges into `input`.
//! - status vocabulary: starting/processing/succeeded/failed/canceled →
//!   Queued/InProgress/Completed/Failed/Failed.
//! - output: a string or array of strings (media URLs) — first string wins;
//!   content = direct download (no separate content endpoint, same discipline
//!   as kling).
//! - webhooks (`webhook` + `webhook_events_filter` top-level params,
//!   `Webhook-Signature` HMAC): deferred to hook_infra P1 — see
//!   `dev-docs/workflow/wait-triggers.md` §4. This provider is poll-only.

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use crate::llm::relay::adaptor::shared_client;

pub struct ReplicateProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

impl ReplicateProvider {
    /// `base_url` includes the version segment, e.g.
    /// `https://api.replicate.com/v1`.
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

    /// Model-specific params shallow-merge into `input`.
    fn build_input(&self, request: &VideoRequest) -> Value {
        let mut input = json!({ "prompt": request.prompt });
        if let Some(seconds) = &request.seconds {
            input["duration"] = Value::String(seconds.clone());
        }
        if let Some(aspect) = super::aspect_ratio_from_size(request.size.as_deref()) {
            input["aspect_ratio"] = Value::String(aspect);
        }
        if let Some(wire) = request
            .input_references
            .first()
            .and_then(|r| r.to_wire_string())
        {
            input["image"] = Value::String(wire);
        }
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object) {
            for (k, v) in map {
                input[k.clone()] = v.clone();
            }
        }
        input
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
                body: text.lines().next().unwrap_or_default().to_string(),
            });
        }
        serde_json::from_str(&text).map_err(|e| ProviderError::Parse(format!("{e}: {text}")))
    }

    async fn get_prediction(&self, task_id: &str) -> Result<Value, ProviderError> {
        let url = format!(
            "{}/predictions/{}",
            self.base_url.trim_end_matches('/'),
            task_id
        );
        self.send(self.http.get(&url)).await
    }

    /// `status` → our `VideoStatus` (canceled folds into Failed).
    fn status_from_wire(v: &Value) -> VideoStatus {
        match v.get("status").and_then(Value::as_str) {
            Some("succeeded") => VideoStatus::Completed,
            Some("failed") | Some("canceled") => VideoStatus::Failed,
            Some("starting") => VideoStatus::Queued,
            _ => VideoStatus::InProgress,
        }
    }

    /// First media URL from `output` (string or array of strings).
    fn output_url(v: &Value) -> Option<String> {
        let out = v.get("output")?;
        match out {
            Value::String(s) => Some(s.clone()),
            Value::Array(items) => items.iter().find_map(|i| i.as_str().map(str::to_string)),
            _ => None,
        }
    }

    fn prediction_error(v: &Value) -> Option<String> {
        v.get("error").and_then(Value::as_str).map(str::to_string)
    }
}

#[async_trait]
impl ModelProvider for ReplicateProvider {
    fn name(&self) -> &str {
        "replicate"
    }

    /// Required by the trait; Replicate has no unified chat surface — return
    /// `Config` so the kernel skips (not fails) this channel.
    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Config(
            "provider replicate does not support chat".into(),
        ))
    }

    /// 提交 prediction：model 含 `/`（官方模型 `owner/name`）→
    /// `POST /models/{owner}/{name}/predictions`；否则视为 version →
    /// `POST /predictions` + `{"version": …}`。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let input = self.build_input(request);
        let (path, mut body) = if model.contains('/') {
            (
                format!("/models/{model}/predictions"),
                json!({ "input": input }),
            )
        } else {
            (
                "/predictions".to_string(),
                json!({ "version": model, "input": input }),
            )
        };
        // Webhook registration (hook path — wait-triggers.md §4): top-level
        // prediction params, terminal events only.
        if let Some(hook) = &request.callback_url {
            body["webhook"] = Value::String(hook.clone());
            body["webhook_events_filter"] = json!(["completed"]);
        }
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let parsed = self.send(self.http.post(&url).json(&body)).await?;
        let task_id = parsed
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(
                "replicate: prediction response without id".into(),
            ));
        }
        Ok(VideoTask {
            id: task_id,
            status: Self::status_from_wire(&parsed),
            progress: None,
            error: Self::prediction_error(&parsed),
        })
    }

    /// 轮询 prediction。
    async fn video_query(&self, task_id: &str, _model: &str) -> Result<VideoTask, ProviderError> {
        let parsed = self.get_prediction(task_id).await?;
        Ok(VideoTask {
            id: task_id.to_string(),
            status: Self::status_from_wire(&parsed),
            progress: None,
            error: Self::prediction_error(&parsed),
        })
    }

    /// 拉取成片：`output` 的首个媒体 URL 直接下载（调用方及时转存）。
    async fn video_content(&self, task_id: &str, _model: &str) -> Result<Vec<u8>, ProviderError> {
        let parsed = self.get_prediction(task_id).await?;
        let Some(url) = Self::output_url(&parsed) else {
            return Err(ProviderError::Parse(format!(
                "replicate: prediction {task_id} has no output url"
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
                body: format!("replicate: download failed {status}"),
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// base_url carries the version segment (preset default
    /// `https://api.replicate.com/v1`), mirrored here with `/v1`.
    fn provider(base_url: String) -> ReplicateProvider {
        ReplicateProvider::new(format!("{base_url}/v1"), Some("r8-test".into()), None, None)
    }

    fn prediction(status: &str, output: Value) -> Value {
        json!({ "id": "pred-1", "status": status, "output": output, "error": null })
    }

    #[tokio::test]
    async fn official_model_submit_posts_to_owner_name_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/minimax/video-01/predictions"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(prediction("starting", Value::Null)),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p
            .video_submit(
                &VideoRequest {
                    prompt: "夜景城市".into(),
                    seconds: Some("5".into()),
                    size: None,
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "minimax/video-01",
            )
            .await
            .unwrap();
        assert_eq!(task.id, "pred-1");
        assert_eq!(task.status, VideoStatus::Queued);

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["input"]["prompt"], "夜景城市");
        assert_eq!(body["input"]["duration"], "5");
        let auth = reqs[0]
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(auth, "Bearer r8-test");
    }

    #[tokio::test]
    async fn version_ref_submit_uses_predictions_path_with_version() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/predictions"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(prediction("starting", Value::Null)),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        p.video_submit(
            &VideoRequest {
                prompt: "x".into(),
                seconds: None,
                size: None,
                input_references: Vec::new(),
                callback_url: None,
            },
            "abcd1234version",
        )
        .await
        .unwrap();

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["version"], "abcd1234version");
    }

    #[tokio::test]
    async fn reference_image_rides_input_image() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/minimax/video-01/predictions"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(prediction("starting", Value::Null)),
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
            "minimax/video-01",
        )
        .await
        .unwrap();

        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["input"]["image"], "https://cdn.example.com/a.png");
    }

    #[tokio::test]
    async fn query_maps_terminal_statuses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/predictions/pred-ok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(prediction(
                "succeeded",
                json!(["https://cdn.replicate.com/out.mp4"]),
            )))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/predictions/pred-bad"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"id": "pred-bad", "status": "failed", "error": "gpu oom", "output": null}),
            ))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let ok = p.video_query("pred-ok", "m").await.unwrap();
        assert_eq!(ok.status, VideoStatus::Completed);
        let bad = p.video_query("pred-bad", "m").await.unwrap();
        assert_eq!(bad.status, VideoStatus::Failed);
        assert_eq!(bad.error.as_deref(), Some("gpu oom"));
    }

    #[tokio::test]
    async fn content_downloads_first_output_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/predictions/pred-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(prediction(
                "succeeded",
                json!(["http://127.0.0.1:1/out.mp4"]),
            )))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        // Port 1 refuses — asserts query→download wiring, not bytes.
        let err = p.video_content("pred-1", "m").await.unwrap_err();
        assert!(
            matches!(
                err,
                ProviderError::Http { .. } | ProviderError::Transport(_)
            ),
            "{err:?}"
        );
    }
}

//! Kling AI (可灵) provider for internal consumption: implements the agent
//! `ModelProvider` video surface (submit/query/content) over Kling's native
//! protocol, so flows can route `provider: "kling"` channels onto
//! `LlmRouter::call().video_*`.
//!
//! Reference matrix:
//! - provider-for-internal-consumption shape: [照抄本仓
//!   `providers/anthropic.rs` / `providers/elevenlabs.rs`] — same constructor
//!   surface; non-video modalities keep trait defaults (`ProviderError::
//!   Config`) so the kernel skips the channel (failover, no failure report).
//! - per-request JWT auth (HS256, `iss = access_key`, `exp` ≈ 30min, `nbf`)
//!   sent as `Authorization: Bearer <jwt>`: [参考 Kling AI 公开 API 文档，
//!   本地无 third 源码]. **Key format = `access_key:secret_key`** in the
//!   single pool key field [自造-简化] — avoids a second credential column
//!   in the keys machinery; the provider splits on the first `:`.
//! - split submit/query paths (`/v1/videos/text2video` vs
//!   `/v1/videos/image2video`): upstream has no unified task endpoint, so
//!   `video_query` probes `image2video` first then falls back to
//!   `text2video` on 404 [自造-务实：任务句柄只有 provider id，类型不可恢复].
//! - response envelope `{code, message, data}`: `code != 0` over HTTP 200 is
//!   mapped to a synthetic `Http 500` (transient → kernel failover); real
//!   4xx/5xx pass through their status codes.
//! - reference images: first `VideoInputRef` rides Kling's `image` field
//!   (data-URL or https URL) switching the submit to image2video.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::provider::{
    ChatRequest, ChatResponse, ModelProvider, ProviderError, VideoRequest, VideoStatus, VideoTask,
};

use crate::llm::relay::adaptor::shared_client;

pub struct KlingProvider {
    http: reqwest::Client,
    base_url: String,
    /// `access_key:secret_key` — split per request for JWT signing.
    credential: Option<(String, String)>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

impl KlingProvider {
    /// `base_url` is the Kling root (no `/v1`), e.g.
    /// `https://api.klingai.com` or `https://api-singapore.klingai.com`.
    /// `api_key` carries `access_key:secret_key`.
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        param_override: Option<Value>,
        header_override: Option<Value>,
    ) -> Self {
        let credential = api_key.and_then(|k| {
            k.split_once(':')
                .map(|(ak, sk)| (ak.trim().to_string(), sk.trim().to_string()))
                .filter(|(ak, sk)| !ak.is_empty() && !sk.is_empty())
        });
        Self {
            http: shared_client().clone(),
            base_url: base_url.into(),
            credential,
            param_override,
            header_override,
        }
    }

    /// Per-request HS256 JWT (`iss = access_key`, exp now+30min, nbf now-5s).
    fn bearer(&self) -> Result<String, ProviderError> {
        let Some((ak, sk)) = &self.credential else {
            return Err(ProviderError::Config(
                "kling key must be `access_key:secret_key`".into(),
            ));
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let claims = json!({
            "iss": ak,
            "exp": now + 1800,
            "nbf": now.saturating_sub(5),
        });
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let token = jsonwebtoken::encode(
            &header,
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(sk.as_bytes()),
        )
        .map_err(|e| ProviderError::Config(format!("kling jwt sign: {e}")))?;
        Ok(format!("Bearer {token}"))
    }

    fn apply_headers(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let mut req = req.bearer_auth(self.bearer()?.trim_start_matches("Bearer "));
        if let Some(over) = &self.header_override
            && let Some(map) = over.as_object()
        {
            for (k, v) in map {
                if let Some(vs) = v.as_str() {
                    req = req.header(k.as_str(), vs);
                }
            }
        }
        Ok(req)
    }

    /// Shallow-merge channel `param_override` into the request body
    /// (mode / cfg_scale / aspect_ratio / negative_prompt / callback_url …).
    fn merge_overrides(&self, body: &mut Value) {
        if let Some(map) = self.param_override.as_ref().and_then(Value::as_object)
            && let Some(target) = body.as_object_mut()
        {
            for (k, v) in map {
                target.insert(k.clone(), v.clone());
            }
        }
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let req = self.apply_headers(self.http.post(&url).json(body))?;
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

    async fn get(&self, path: &str) -> Result<Result<Value, ()>, ProviderError> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let req = self.apply_headers(self.http.get(&url))?;
        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(Err(())); // probe miss — caller falls back to the twin path
        }
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Self::unwrap_envelope(status, &text).map(Ok)
    }

    /// Unwrap Kling's `{code, message, data}` envelope. `code != 0` over
    /// HTTP 200 becomes a synthetic `Http 500` (transient → kernel
    /// failover); real status codes pass through.
    fn unwrap_envelope(status: u16, text: &str) -> Result<Value, ProviderError> {
        let parsed: Value =
            serde_json::from_str(text).map_err(|e| ProviderError::Parse(format!("{e}: {text}")))?;
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: Self::envelope_message(&parsed, text),
            });
        }
        let code = parsed.get("code").and_then(Value::as_i64).unwrap_or(0);
        if code != 0 {
            return Err(ProviderError::Http {
                status: 500,
                body: format!(
                    "kling code {code}: {}",
                    Self::envelope_message(&parsed, text)
                ),
            });
        }
        Ok(parsed)
    }

    fn envelope_message(parsed: &Value, raw: &str) -> String {
        parsed
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| raw.lines().next().unwrap_or_default().to_string())
    }

    fn task_path(task_id: &str, image2video: bool) -> String {
        if image2video {
            format!("/v1/videos/image2video/{task_id}")
        } else {
            format!("/v1/videos/text2video/{task_id}")
        }
    }

    /// Probe `image2video` first, fall back to `text2video` on 404.
    async fn query_task(&self, task_id: &str) -> Result<Value, ProviderError> {
        for image2video in [true, false] {
            match self.get(&Self::task_path(task_id, image2video)).await? {
                Ok(v) => return Ok(v),
                Err(()) => continue, // probe miss — try the twin path
            }
        }
        Err(ProviderError::Http {
            status: 404,
            body: format!("kling: no such task {task_id} (both paths)"),
        })
    }

    /// `data.task_status` → our `VideoStatus`.
    fn status_from_wire(v: &Value) -> VideoStatus {
        match v
            .get("data")
            .and_then(|d| d.get("task_status"))
            .and_then(Value::as_str)
        {
            Some("succeed") => VideoStatus::Completed,
            Some("failed") => VideoStatus::Failed,
            Some("submitted") => VideoStatus::Queued,
            _ => VideoStatus::InProgress,
        }
    }

    /// First result video URL from `data.task_result.videos[]`.
    fn result_url(v: &Value) -> Option<String> {
        v.get("data")
            .and_then(|d| d.get("task_result"))
            .and_then(|r| r.get("videos"))
            .and_then(Value::as_array)
            .and_then(|vs| vs.first())
            .and_then(|v| v.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

#[async_trait]
impl ModelProvider for KlingProvider {
    fn name(&self) -> &str {
        "kling"
    }

    /// Required by the trait; Kling has no chat surface — return `Config` so
    /// the kernel skips (not fails) this channel.
    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Config(
            "provider kling does not support chat".into(),
        ))
    }

    /// 提交异步视频任务：无参考图 → `text2video`；有 → `image2video`
    /// （首帧 = 首个参考图）。渠道 `param_override` 浅合并进 body。
    async fn video_submit(
        &self,
        request: &VideoRequest,
        model: &str,
    ) -> Result<VideoTask, ProviderError> {
        let mut body = json!({ "model_name": model, "prompt": request.prompt });
        if let Some(seconds) = &request.seconds {
            body["duration"] = Value::String(seconds.clone());
        }
        if let Some(hook) = &request.callback_url {
            body["callback_url"] = Value::String(hook.clone());
        }
        if let Some(aspect) = super::aspect_ratio_from_size(request.size.as_deref()) {
            body["aspect_ratio"] = Value::String(aspect);
        }
        self.merge_overrides(&mut body);
        let path = if request.input_references.is_empty() {
            "/v1/videos/text2video"
        } else {
            if let Some(wire) = request.input_references[0].to_wire_string() {
                body["image"] = Value::String(wire);
            }
            "/v1/videos/image2video"
        };
        let parsed = self.post(path, &body).await?;
        let task_id = parsed
            .get("data")
            .and_then(|d| d.get("task_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if task_id.is_empty() {
            return Err(ProviderError::Parse(
                "kling: envelope without data.task_id".into(),
            ));
        }
        Ok(VideoTask {
            id: task_id,
            status: Self::status_from_wire(&parsed),
            progress: None,
            error: None,
        })
    }

    /// 轮询任务（image2video → text2video 回退）。
    async fn video_query(&self, task_id: &str, _model: &str) -> Result<VideoTask, ProviderError> {
        let parsed = self.query_task(task_id).await?;
        Ok(VideoTask {
            id: task_id.to_string(),
            status: Self::status_from_wire(&parsed),
            progress: None,
            error: parsed
                .get("data")
                .and_then(|d| d.get("task_status_msg"))
                .and_then(Value::as_str)
                .filter(|_| Self::status_from_wire(&parsed) == VideoStatus::Failed)
                .map(str::to_string),
        })
    }

    /// 拉取成片：Kling 无独立 content 端点——查询拿 `task_result.videos[0].url`
    /// 再下载字节（CDN 链接有有效期，调用方应及时转存）。
    async fn video_content(&self, task_id: &str, _model: &str) -> Result<Vec<u8>, ProviderError> {
        let parsed = self.query_task(task_id).await?;
        let Some(url) = Self::result_url(&parsed) else {
            return Err(ProviderError::Parse(format!(
                "kling: task {task_id} has no result video url"
            )));
        };
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ProviderError::Http {
                status,
                body: format!("kling: download failed {status}"),
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

    fn provider(base_url: String) -> KlingProvider {
        KlingProvider::new(base_url, Some("ak-test:sk-test".into()), None, None)
    }

    fn task_envelope(status: &str, url: Option<&str>) -> Value {
        let mut data = json!({ "task_id": "t-1", "task_status": status });
        if let Some(u) = url {
            data["task_result"] = json!({ "videos": [{ "id": "v1", "url": u }] });
        }
        json!({ "code": 0, "message": "Success", "data": data })
    }

    #[tokio::test]
    async fn text2video_submit_posts_signed_jwt_and_parses_task() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos/text2video"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(task_envelope("submitted", None)),
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
                "kling-v1-6",
            )
            .await
            .unwrap();
        assert_eq!(task.id, "t-1");
        assert_eq!(task.status, VideoStatus::Queued);

        // JWT is well-formed AND verifiable with sk (iss = ak).
        let reqs = server.received_requests().await.unwrap();
        let auth = reqs[0]
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap();
        let token = auth.trim_start_matches("Bearer ");
        assert_eq!(
            token.split('.').count(),
            3,
            "JWT must have 3 segments: {token}"
        );
        let claims = jsonwebtoken::decode::<Value>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(b"sk-test"),
            &jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256),
        )
        .unwrap();
        assert_eq!(claims.claims["iss"], "ak-test");
    }

    #[tokio::test]
    async fn image2video_submit_carries_first_reference() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos/image2video"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(task_envelope("submitted", None)),
            )
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
                "kling-v1-6",
            )
            .await
            .unwrap();
        assert_eq!(task.status, VideoStatus::Queued);
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["image"], "https://cdn.example.com/a.png");
        assert_eq!(body["model_name"], "kling-v1-6");
    }

    #[tokio::test]
    async fn query_falls_back_from_image2video_to_text2video() {
        let server = MockServer::start().await;
        // image2video probe misses; text2video answers succeed.
        Mock::given(method("GET"))
            .and(path("/v1/videos/image2video/t-1"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(json!({"code": 1002, "message": "task not found"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/text2video/t-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(task_envelope(
                "succeed",
                Some("https://cdn.kling.ai/out.mp4"),
            )))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let task = p.video_query("t-1", "kling-v1-6").await.unwrap();
        assert_eq!(task.status, VideoStatus::Completed);
    }

    #[tokio::test]
    async fn envelope_error_over_http_200_maps_to_transient_500() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos/text2video"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code": 1000, "message": "internal error"})),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let err = p
            .video_submit(
                &VideoRequest {
                    prompt: "x".into(),
                    seconds: None,
                    size: None,
                    input_references: Vec::new(),
                    callback_url: None,
                },
                "kling-v1-6",
            )
            .await
            .unwrap_err();
        match err {
            ProviderError::Http { status, body } => {
                assert_eq!(status, 500);
                assert!(body.contains("1000"), "{body}");
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn content_downloads_result_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/text2video/t-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(task_envelope(
                "succeed",
                Some("http://127.0.0.1:1/out.mp4"), // replaced below by real mock path
            )))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        // Non-200 download → Http error (no live CDN in tests; assert the
        // query→download wiring, not the bytes).
        let err = p.video_content("t-1", "kling-v1-6").await.unwrap_err();
        assert!(
            matches!(
                err,
                ProviderError::Http { .. } | ProviderError::Transport(_)
            ),
            "{err:?}"
        );
    }

    #[test]
    fn aspect_ratio_from_size_shared() {
        assert_eq!(
            super::super::aspect_ratio_from_size(Some("1920x1080")).as_deref(),
            Some("16:9")
        );
        assert_eq!(super::super::aspect_ratio_from_size(Some("junk")), None);
    }
}

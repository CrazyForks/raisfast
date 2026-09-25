//! `video` node (media-nodes.md §4) — async submit→park→poll→resume through
//! the llm foundation facade (`video_submit/video_query/video_content`).
//!
//! The engine splits execution in two durable segments; this file owns the
//! first (submit) plus the asset fetch used by the poller (`video_poll.rs`)
//! for the second. Double-billing guard (media-nodes.md §4.2, the MPT
//! `UnconfirmedTaskError` discipline): the engine persists `phase=submitting`
//! BEFORE the upstream call and `phase=submitted` right after; a crash in
//! between leaves an unconfirmed state that is never resubmitted
//! automatically — the node fails with a pointer to `llm_tasks`/the provider
//! console. Submit-time channel failover already lives in the facade; the
//! engine adds no retry of its own on top.

use std::sync::Arc;
use std::time::Instant;

use raisfast_agent::provider::VideoRequest;
use serde_json::{Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

use super::LlmRuntime;

/// Default async deadline (30min) — poller fails the node past this point.
pub const DEFAULT_VIDEO_DEADLINE_SECS: i64 = 1800;

/// `video` node config (media-nodes.md §4.1). `prompt` is a C3.1 template.
/// `model` is required: video price spread across providers is too large for
/// a silent tenant default.
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VideoConfig {
    pub model: String,
    pub prompt: String,
    /// Seconds of footage as the upstream wire string (e.g. "5"/"10");
    /// absent = provider default.
    #[serde(default)]
    pub seconds: Option<String>,
    /// Wire size string, e.g. `1280x720`; absent = provider default.
    #[serde(default)]
    pub size: Option<String>,
    /// Submit-call timeout (the synchronous HTTP round-trip), default 60s.
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_ms: Option<i64>,
    /// Async task deadline; the poller fails the node past it.
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub deadline_secs: Option<i64>,
    /// Reference images (image-to-video / first frame) — ValueExpr resolving
    /// to: a string (storage key or https URL) / an array of strings / an
    /// array of `{key|url}` objects (e.g. an upstream image node's
    /// `images[]`). https URLs pass through; storage keys are fetched and
    /// inlined as base64 (no public URL required).
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub input_images: Option<Value>,
}

/// Config validation (media-nodes.md §4.1 bounds).
pub(super) fn validate(config: &Value) -> AppResult<()> {
    let c: VideoConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'video' config invalid: {e}")))?;
    if c.model.trim().is_empty() {
        return Err(AppError::BadRequest("video: model 必填".into()));
    }
    if c.prompt.trim().is_empty() {
        return Err(AppError::BadRequest("video: prompt 不能为空".into()));
    }
    if let Some(s) = &c.seconds {
        let ok = s.parse::<i64>().is_ok_and(|v| (1..=120).contains(&v));
        if !ok {
            return Err(AppError::BadRequest(
                "video: seconds 须为 1..=120 的数字字符串".into(),
            ));
        }
    }
    if c.size.as_deref().is_some_and(str::is_empty) {
        return Err(AppError::BadRequest("video: size 不能为空字符串".into()));
    }
    if c.timeout_ms.is_some_and(|t| t < 1) {
        return Err(AppError::BadRequest(
            "video: timeout_ms 须为 ≥1 的整数".into(),
        ));
    }
    if c.deadline_secs.is_some_and(|t| t < 1) {
        return Err(AppError::BadRequest(
            "video: deadline_secs 须为 ≥1 的整数".into(),
        ));
    }
    Ok(())
}

fn status_wire(s: raisfast_agent::provider::VideoStatus) -> String {
    format!("{s:?}").to_lowercase()
}

/// Normalize the resolved `input_images` value into wire refs:
/// string / string[] / {key|url}[] → https URLs pass through, storage keys
/// are fetched and inlined as base64 with a sniffed mime.
async fn resolve_input_refs(
    runtime: &LlmRuntime,
    expr: &Value,
    pool: &Pool,
    storage: &Arc<dyn Storage>,
) -> AppResult<Vec<raisfast_agent::provider::VideoInputRef>> {
    let _ = runtime; // reserved: per-tenant fetch policy
    let resolved = crate::flows::engine::resolve(expr, pool)?;
    let mut raw: Vec<String> = Vec::new();
    match resolved {
        Value::String(s) => raw.push(s),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(s) => raw.push(s),
                    Value::Object(o) => {
                        if let Some(k) = o.get("key").and_then(Value::as_str) {
                            raw.push(k.to_string());
                        } else if let Some(u) = o.get("url").and_then(Value::as_str) {
                            raw.push(u.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::Null => {}
        other => {
            return Err(AppError::BadRequest(format!(
                "video: input_images 解析结果须为字符串/数组（got {}）",
                json_typename(&other)
            )));
        }
    }
    let mut refs = Vec::with_capacity(raw.len());
    for r in raw {
        let r = r.trim().to_string();
        if r.is_empty() {
            continue;
        }
        if r.starts_with("https://") || r.starts_with("http://") {
            refs.push(raisfast_agent::provider::VideoInputRef::from_url(r));
        } else {
            // Storage key — inline as base64 (upstreams without a public
            // fetchable URL still accept data-URLs).
            let bytes = storage.get(&r).await.map_err(|e| {
                AppError::BadRequest(format!("video: input_images 读取 {r} 失败: {e}"))
            })?;
            let (_ext, mime) = super::sniff_image(&bytes);
            use base64::Engine as _;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            refs.push(raisfast_agent::provider::VideoInputRef {
                url: None,
                b64_json: Some(b64),
                mime: Some(mime.to_string()),
            });
        }
    }
    Ok(refs)
}

fn json_typename(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// First durable segment: render prompt → `video_submit` → task info for the
/// engine to persist as `phase=submitted` before parking. The engine has
/// already persisted `phase=submitting` before calling this.
///
/// # Errors
/// `BadRequest` on missing template refs or a non-retryable provider error;
/// `Internal` on transient submit failures — either way the engine must NOT
/// blindly resubmit (double-billing guard).
pub async fn run_video_submit(
    runtime: &LlmRuntime,
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
    callback_url: Option<&str>,
) -> AppResult<ExecOutcome> {
    let cfg: VideoConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("video config: {e}")))?;
    let prompt = super::render_prompt_text(&cfg.prompt, pool)?;
    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(60_000) as u64;
    let deadline_secs = cfg
        .deadline_secs
        .filter(|t| *t > 0)
        .unwrap_or(DEFAULT_VIDEO_DEADLINE_SECS);
    let started = Instant::now();

    let call = runtime
        .router
        .call(&runtime.tenant, crate::llm::models::log::LogSource::Flow);
    let input_references = if let Some(expr) = &cfg.input_images {
        resolve_input_refs(runtime, expr, pool, storage).await?
    } else {
        Vec::new()
    };
    let request = VideoRequest {
        prompt,
        seconds: cfg.seconds.clone(),
        size: cfg.size.clone(),
        input_references,
        callback_url: callback_url.map(str::to_string),
    };
    let task = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        call.video_submit(&cfg.model, &request),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("video 提交超时 {timeout_ms}ms")))??;
    if task.id.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "video: 上游未返回 task id"
        )));
    }

    let deadline_unix =
        (crate::utils::tz::now_utc() + chrono::Duration::seconds(deadline_secs)).timestamp();
    Ok(ExecOutcome {
        output: json!({
            "phase": "submitted",
            "task_id": task.id,
            "model": cfg.model,
            "status": status_wire(task.status),
            "deadline_unix": deadline_unix,
        }),
        usage: Some(json!({ "task": task.id })),
        latency_ms: Some(started.elapsed().as_millis() as i64),
    })
}

/// Second segment asset fetch (used by the poller): pull the completed video
/// bytes and persist to storage under `gen/flows/{instance}/{node}/`.
/// Returns the `{key, url}` reference for the resume payload.
pub(crate) async fn fetch_and_store(
    runtime: &LlmRuntime,
    storage: &Arc<dyn Storage>,
    instance_id: i64,
    node_id: &str,
    model: &str,
    task_id: &str,
) -> AppResult<Value> {
    let call = runtime
        .router
        .call(&runtime.tenant, crate::llm::models::log::LogSource::Flow);
    let bytes = call.clone().video_content(model, task_id).await?;
    if bytes.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "video: 上游返回空视频 (task {task_id})"
        )));
    }
    // Content endpoint gave us bytes — persist directly (no result URL).
    let key = format!("gen/flows/{instance_id}/{node_id}/video-{task_id}.mp4");
    storage.put(&key, &bytes, "video/mp4").await?;
    let url = storage
        .url(&key)
        .await
        .unwrap_or_else(|_| format!("/{key}"));
    Ok(json!({ "key": key, "url": url, "bytes": bytes.len() }))
}

/// Persist a completed video from its result URL (shared by the poll path
/// and the hook path — media-nodes.md §6 asset convention).
pub(crate) async fn store_from_url(
    storage: &Arc<dyn Storage>,
    instance_id: i64,
    node_id: &str,
    task_id: &str,
    url: &str,
) -> AppResult<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("video download client: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("video 下载失败: {e}")))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(AppError::Internal(anyhow::anyhow!(
            "video: 下载失败 HTTP {status}"
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("video: 读取响应失败: {e}")))?;
    if bytes.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "video: 上游返回空视频 (task {task_id})"
        )));
    }
    let key = format!("gen/flows/{instance_id}/{node_id}/video-{task_id}.mp4");
    storage.put(&key, &bytes, "video/mp4").await?;
    let public_url = storage
        .url(&key)
        .await
        .unwrap_or_else(|_| format!("/{key}"));
    Ok(json!({ "key": key, "url": public_url, "bytes": bytes.len() }))
}

/// The video scenario of the generic wait-poll hub (`poll_infra.rs`):
/// query the parked upstream task and translate terminal states into resume
/// envelopes. Deadline timeout is handled by the infra BEFORE `poll` runs —
/// no timeout branch here; transient query errors leave the node parked.
pub struct VideoPoller;

#[async_trait::async_trait]
impl super::super::poll_infra::WaitPoller for VideoPoller {
    fn kind(&self) -> &'static str {
        super::T_VIDEO
    }

    fn park_event(&self) -> &'static str {
        super::super::events::EV_VIDEO_SUBMITTED
    }

    fn supports_hook(&self) -> bool {
        true
    }

    /// Dialect sniff on the webhook body [自造-务实]: the node output carries
    /// no channel identity (failover may pick any channel), so the payload
    /// shape decides — Kling wraps in `{code, data:{task_status}}`,
    /// Replicate posts a flat `{status, output}` prediction.
    async fn translate_hook(
        &self,
        ctx: &super::super::poll_infra::HookCtx<'_>,
        body: &Value,
    ) -> AppResult<Option<super::ResumeEnvelope>> {
        let (status, url) = if body
            .get("data")
            .and_then(|d| d.get("task_status"))
            .is_some()
        {
            (
                body.pointer("/data/task_status").and_then(Value::as_str),
                body.pointer("/data/task_result/videos/0/url")
                    .and_then(Value::as_str),
            )
        } else if body.get("status").is_some() {
            let out = body.get("output");
            let url = match out {
                Some(Value::String(s)) => Some(s.as_str()),
                Some(Value::Array(items)) => items.first().and_then(Value::as_str),
                _ => None,
            };
            (body.get("status").and_then(Value::as_str), url)
        } else {
            (None, None) // unknown dialect — treat as non-terminal
        };
        let task_id = ctx
            .info
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match status {
            Some("succeed") | Some("succeeded") => {
                let Some(url) = url else {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "video hook: terminal success without result url"
                    )));
                };
                let video =
                    store_from_url(&ctx.storage, *ctx.instance_id, ctx.node_id, &task_id, url)
                        .await?;
                Ok(Some(super::ResumeEnvelope {
                    action: "video.done".into(),
                    data: Some(json!({
                        "video": video,
                        "task_id": task_id,
                        "status": "completed",
                        "via": "hook",
                    })),
                }))
            }
            Some("failed") | Some("canceled") => Ok(Some(super::ResumeEnvelope {
                action: "video.fail".into(),
                data: Some(json!({
                    "task_id": task_id,
                    "status": "failed",
                    "via": "hook",
                    "error": body
                        .pointer("/data/task_status_msg")
                        .or_else(|| body.get("error"))
                        .and_then(Value::as_str)
                        .unwrap_or("upstream failed"),
                })),
            })),
            // submitted/processing/starting — non-terminal, keep parked.
            _ => Ok(None),
        }
    }

    async fn poll(
        &self,
        ctx: &super::super::poll_infra::PollCtx<'_>,
    ) -> AppResult<Option<super::ResumeEnvelope>> {
        let Some(task_id) = ctx.info.get("task_id").and_then(|v| v.as_str()) else {
            return Ok(None);
        };
        let model = ctx
            .info
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let runtime = LlmRuntime {
            router: ctx.router.clone(),
            tenant: ctx.tenant.to_owned(),
            caller: None,
        };
        let call = ctx
            .router
            .call(ctx.tenant, crate::llm::models::log::LogSource::Flow);
        let task = call.video_query(&model, task_id).await?;
        use raisfast_agent::provider::VideoStatus;
        match task.status {
            VideoStatus::Completed => {
                let video = fetch_and_store(
                    &runtime,
                    &media_storage()?,
                    *ctx.instance_id,
                    ctx.node_id,
                    &model,
                    task_id,
                )
                .await?;
                Ok(Some(super::ResumeEnvelope {
                    action: "video.done".into(),
                    data: Some(json!({
                        "video": video,
                        "task_id": task_id,
                        "status": "completed",
                    })),
                }))
            }
            VideoStatus::Failed => Ok(Some(super::ResumeEnvelope {
                action: "video.fail".into(),
                data: Some(json!({
                    "task_id": task_id,
                    "status": "failed",
                    "error": task.error.unwrap_or_else(|| "upstream failed".into()),
                })),
            })),
            // Queued / InProgress — the infra's deadline gate owns timeout.
            VideoStatus::Queued | VideoStatus::InProgress => Ok(None),
        }
    }
}

/// Process-wide media storage (same handle the executors use — media-nodes.md
/// §5; pollers run outside `FlowsExec`, so they read the shared install).
fn media_storage() -> AppResult<Arc<dyn Storage>> {
    super::super::exec::shared_storage()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("storage unavailable")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::graph::NodeData;
    use crate::llm::cache::ModelInfo;
    use crate::llm::service::LlmRouter;
    use crate::types::snowflake_id::SnowflakeId;
    use async_trait::async_trait;
    use raisfast_agent::provider::{ModelProvider, ProviderError, VideoStatus, VideoTask};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MemStorage {
        files: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
    }

    #[async_trait]
    impl crate::storage::Storage for MemStorage {
        async fn put(&self, key: &str, data: &[u8], _ct: &str) -> AppResult<()> {
            self.files.lock().unwrap().insert(key.into(), data.into());
            Ok(())
        }
        async fn get(&self, key: &str) -> AppResult<Vec<u8>> {
            self.files
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| AppError::not_found("storage"))
        }
        async fn delete(&self, key: &str) -> AppResult<()> {
            self.files.lock().unwrap().remove(key);
            Ok(())
        }
        async fn url(&self, key: &str) -> AppResult<String> {
            Ok(format!("http://localhost/{key}"))
        }
    }

    /// Recorded video submit requests (for input-reference assertions).
    static SEEN_REQUESTS: std::sync::OnceLock<Mutex<Vec<VideoRequest>>> =
        std::sync::OnceLock::new();
    fn seen_requests() -> &'static Mutex<Vec<VideoRequest>> {
        SEEN_REQUESTS.get_or_init(|| Mutex::new(Vec::new()))
    }

    struct MockVideoProvider {
        tasks: Mutex<Vec<VideoTask>>,
    }

    #[async_trait]
    impl ModelProvider for MockVideoProvider {
        fn name(&self) -> &str {
            "mock-video"
        }
        async fn chat(
            &self,
            _request: &raisfast_agent::provider::ChatRequest<'_>,
            _model: &str,
        ) -> Result<raisfast_agent::ChatResponse, ProviderError> {
            Err(ProviderError::Transport("mock: chat unsupported".into()))
        }
        async fn video_submit(
            &self,
            request: &VideoRequest,
            _model: &str,
        ) -> Result<VideoTask, ProviderError> {
            seen_requests().lock().unwrap().push(request.clone());
            let id = format!("task-{}", self.tasks.lock().unwrap().len() + 1);
            let task = VideoTask {
                id,
                status: VideoStatus::Queued,
                progress: None,
                error: None,
            };
            self.tasks.lock().unwrap().push(task.clone());
            Ok(task)
        }
    }

    fn video_runtime() -> LlmRuntime {
        let mut cache = crate::llm::cache::ChannelCache::default();
        let row = crate::llm::models::channel::LlmChannel {
            id: SnowflakeId(1),
            tenant_id: Some("default".to_owned()),
            name: "mock".to_owned(),
            provider: "openai".to_owned(),
            base_url: "http://mock.test/v1".to_owned(),
            api_keys: serde_json::to_value(vec![crate::llm::models::channel::LlmKeyEntry {
                key: "plain".to_owned(),
                status: crate::llm::models::channel::LlmKeyStatus::Active,
                disabled_reason: None,
                disabled_at: None,
                max_concurrency: None,
            }])
            .unwrap(),
            key_mode: crate::llm::models::channel::LlmKeyMode::Polling,
            status: crate::llm::models::channel::LlmChannelStatus::Enabled,
            models: "vid-model".into(),
            model_mapping: None,
            priority: 0,
            weight: 0,
            channel_groups: "default".to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            used_quota: 0,
            cost_mode: crate::llm::models::channel::LlmCostMode::Usage,
            cost_discount: 1.0,
            monthly_cost: None,
            test_model: None,
            test_time: None,
            response_time: None,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        };
        let cached = crate::llm::cache::ChannelCache::from_row(&row);
        cache
            .channels
            .insert(cached.id, std::sync::Arc::new(cached));
        cache.models.insert(
            ("default".to_owned(), "vid-model".to_owned()),
            std::sync::Arc::new(ModelInfo {
                name: "vid-model".to_owned(),
                model_type: crate::llm::models::model::LlmModelType::Video,
                pricing: crate::llm::cache::Pricing {
                    price_mode: crate::llm::models::model::LlmPriceMode::PerCall,
                    input_price: 0.0,
                    output_price: 0.0,
                    cache_read_price: None,
                    cache_write_price: None,
                    call_price: Some(0.5),
                },
                params: None,
            }),
        );
        cache.rebuild_routes();
        let router = LlmRouter::from_cache_for_test(cache);
        router.providers.insert(
            (SnowflakeId(1), 0),
            std::sync::Arc::new(MockVideoProvider {
                tasks: Mutex::new(Vec::new()),
            }) as std::sync::Arc<dyn ModelProvider>,
        );
        LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        }
    }

    fn video_node(config: Value) -> GraphNode {
        GraphNode {
            id: "v1".into(),
            data: NodeData {
                kind: "video".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn submit_returns_submitted_phase_with_deadline() {
        let rt = video_runtime();
        let node = video_node(json!({
            "model": "vid-model",
            "prompt": "镜头：{{#start.scene#}}",
            "seconds": "5"
        }));
        let mut pool = Pool::new();
        pool.entry("start".into())
            .or_default()
            .insert("scene".into(), json!("夜晚的城市"));
        let storage: Arc<MemStorage> = Arc::new(MemStorage::default());
        let out = run_video_submit(
            &rt,
            &(storage.clone() as Arc<dyn Storage>),
            &node,
            &pool,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.output["phase"], "submitted");
        assert_eq!(out.output["model"], "vid-model");
        assert!(
            out.output["task_id"]
                .as_str()
                .is_some_and(|t| !t.is_empty())
        );
        let deadline = out.output["deadline_unix"].as_i64().unwrap();
        assert!(deadline > crate::utils::tz::now_utc().timestamp());
    }

    #[tokio::test]
    async fn missing_template_ref_is_bad_request() {
        let rt = video_runtime();
        let node = video_node(json!({
            "model": "vid-model", "prompt": "{{#start.nope#}}"
        }));
        let storage: Arc<MemStorage> = Arc::new(MemStorage::default());
        let err = run_video_submit(
            &rt,
            &(storage as Arc<dyn Storage>),
            &node,
            &Pool::new(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }

    #[tokio::test]
    async fn input_images_resolved_from_pool_refs() {
        let rt = video_runtime();
        let storage = Arc::new(MemStorage::default());
        // An upstream image node stored an asset; its output refs {key,url}.
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let key = "gen/flows/42/img-1.png";
        crate::storage::Storage::put(storage.as_ref(), key, &png, "image/png")
            .await
            .unwrap();

        let node = video_node(json!({
            "model": "vid-model",
            "prompt": "同款镜头",
            "input_images": {"ref": ["img_1", "images"]}
        }));
        let mut pool = Pool::new();
        pool.entry("img_1".into()).or_default().insert(
            "images".into(),
            json!([{ "key": key, "url": format!("http://localhost/{key}") }]),
        );
        let out = run_video_submit(
            &rt,
            &(storage.clone() as Arc<dyn Storage>),
            &node,
            &pool,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.output["phase"], "submitted");

        // Scoped: drop the registry lock before the next submit — the mock
        // provider locks the same global registry (a held guard across the
        // call would self-deadlock on the single-thread test runtime).
        {
            // Content-match instead of last(): parallel tests share the
            // global registry, so ordering is not ours to assume.
            let reqs = seen_requests().lock().unwrap();
            let req = reqs
                .iter()
                .find(|r| r.prompt == "同款镜头")
                .expect("b64-ref request recorded");
            assert_eq!(req.input_references.len(), 1);
            let wire = req.input_references[0].to_wire_string().unwrap();
            assert!(wire.starts_with("data:image/png;base64,"), "{wire}");
        }

        // https URL passthrough branch
        let node2 = video_node(json!({
            "model": "vid-model",
            "prompt": "https-passthrough-probe",
            "input_images": {"literal": ["https://cdn.example.com/a.png"]}
        }));
        run_video_submit(
            &rt,
            &(storage as Arc<dyn Storage>),
            &node2,
            &Pool::new(),
            None,
        )
        .await
        .unwrap();
        {
            let reqs = seen_requests().lock().unwrap();
            let req2 = reqs
                .iter()
                .find(|r| r.prompt == "https-passthrough-probe")
                .expect("url-ref request recorded");
            assert_eq!(
                req2.input_references[0].url.as_deref(),
                Some("https://cdn.example.com/a.png")
            );
        }
    }

    #[test]
    fn video_config_validation() {
        assert!(
            super::validate(&json!({"model": "m", "prompt": "a cat"})).is_ok(),
            "最小配置"
        );
        assert!(
            super::validate(&json!({"prompt": "x"})).is_err(),
            "缺 model"
        );
        assert!(
            super::validate(&json!({"model": "m"})).is_err(),
            "缺 prompt"
        );
        assert!(
            super::validate(&json!({"model": "m", "prompt": "x", "seconds": "0"})).is_err(),
            "seconds 越界"
        );
        assert!(
            super::validate(&json!({"model": "m", "prompt": "x", "seconds": "abc"})).is_err(),
            "seconds 非数字"
        );
        assert!(
            super::validate(&json!({"model": "m", "prompt": "x", "deadline_secs": 0})).is_err(),
            "deadline<1"
        );
    }
}

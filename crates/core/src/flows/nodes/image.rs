//! `image` node executor (media-nodes.md §2) — synchronous text-to-image
//! through the llm foundation facade (`LlmRouter::call().image()`): routing,
//! key pool, failover, per-image billing and the `Image` model-type gate all
//! live in the kernel. Generated images are persisted to storage
//! (`gen/flows/{run}/{node-seq}/…`, docparse asset convention) and referenced
//! by `{key, url}` — upstream URLs are transient signed addresses and are
//! never forwarded into the pool raw.

use std::sync::Arc;
use std::time::Instant;

use raisfast_agent::provider::ImageRequest;
use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

use super::LlmRuntime;

/// `image` node config (media-nodes.md §2.1). `prompt` is a C3.1 template;
/// error handling stays orthogonal via `modifiers.on_error_strategy` (C1.4).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ImageConfig {
    /// Image model; absent → tenant option `llm.default_image_model`.
    pub model: Option<String>,
    /// Prompt template (`{{#ns.field#}}` refs allowed).
    pub prompt: String,
    /// Number of images (1–10; wire default 1).
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub n: Option<i64>,
    /// Wire size string, e.g. `1024x1024` (None = provider default).
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_ms: Option<i64>,
}

/// Config validation (media-nodes.md §2.1 bounds).
pub(super) fn validate(config: &Value) -> AppResult<()> {
    let c: ImageConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'image' config invalid: {e}")))?;
    if c.prompt.trim().is_empty() {
        return Err(AppError::BadRequest("image: prompt 不能为空".into()));
    }
    if c.n.is_some_and(|n| !(1..=10).contains(&n)) {
        return Err(AppError::BadRequest("image: n 须在 1..=10".into()));
    }
    if let Some(s) = &c.size
        && s.trim().is_empty()
    {
        return Err(AppError::BadRequest("image: size 不能为空字符串".into()));
    }
    if c.timeout_ms.is_some_and(|t| t < 1) {
        return Err(AppError::BadRequest(
            "image: timeout_ms 须为 ≥1 的整数".into(),
        ));
    }
    Ok(())
}

/// Persist one generated image and return its `{key, url}` reference.
async fn persist_image(
    storage: &Arc<dyn Storage>,
    run_id: &str,
    index: usize,
    b64: Option<&str>,
    url: Option<&str>,
) -> AppResult<Value> {
    let bytes: Vec<u8> = if let Some(b64) = b64 {
        base64_decode(b64)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("image: 上游 b64 解码失败: {e}")))?
    } else if let Some(url) = url {
        super::download_https(url).await?
    } else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "image: 上游响应既无 b64_json 也无 url"
        )));
    };
    let (ext, mime) = super::sniff_image(&bytes);
    let key = format!("gen/flows/{run_id}/img-{index}.{ext}");
    storage.put(&key, &bytes, mime).await?;
    let public_url = storage
        .url(&key)
        .await
        .unwrap_or_else(|_| format!("/{key}"));
    Ok(json!({ "key": key, "url": public_url }))
}

fn base64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s.trim())
}

/// Execute the `image` node against the variable pool.
///
/// # Errors
/// `BadRequest` on missing template refs or a non-retryable provider error;
/// `Internal` on transient provider/timeout/storage failures.
pub async fn run_image(
    runtime: &LlmRuntime,
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: ImageConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("image config: {e}")))?;
    let prompt = super::render_prompt_text(&cfg.prompt, pool)?;
    let n = cfg.n.unwrap_or(1).clamp(1, 10) as u32;
    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(120_000) as u64;
    let started = Instant::now();

    let call = runtime
        .router
        .call(&runtime.tenant, crate::llm::models::log::LogSource::Flow);
    let request = ImageRequest {
        prompt,
        n,
        size: cfg.size.clone(),
    };
    let images = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        call.image(cfg.model.as_deref(), &request),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("image 超时 {timeout_ms}ms")))??;

    let run_id = crate::utils::id::new_id().to_string();
    let mut refs = Vec::with_capacity(images.len());
    for (i, img) in images.iter().enumerate() {
        refs.push(
            persist_image(
                storage,
                &run_id,
                i + 1,
                img.b64_json.as_deref(),
                img.url.as_deref(),
            )
            .await?,
        );
    }

    let latency_ms = started.elapsed().as_millis() as i64;
    let mut out = Map::new();
    out.insert("images".into(), Value::Array(refs));
    out.insert("model".into(), json!(cfg.model));
    out.insert("n".into(), json!(images.len()));
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: Some(json!({ "images": images.len() })),
        latency_ms: Some(latency_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::graph::NodeData;
    use crate::llm::cache::ModelInfo;
    use crate::llm::service::LlmRouter;
    use crate::types::snowflake_id::SnowflakeId;
    use async_trait::async_trait;
    use raisfast_agent::provider::{GeneratedImage, ModelProvider, ProviderError};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// HashMap-backed storage mock recording puts.
    #[derive(Debug, Default)]
    struct MemStorage {
        files: Mutex<HashMap<String, Vec<u8>>>,
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

    struct MockImageProvider {
        images: Vec<GeneratedImage>,
    }

    #[async_trait]
    impl ModelProvider for MockImageProvider {
        fn name(&self) -> &str {
            "mock-image"
        }
        async fn chat(
            &self,
            _request: &raisfast_agent::provider::ChatRequest<'_>,
            _model: &str,
        ) -> Result<raisfast_agent::ChatResponse, ProviderError> {
            Err(ProviderError::Transport("mock: chat unsupported".into()))
        }
        async fn generate_image(
            &self,
            _request: &ImageRequest,
            _model: &str,
        ) -> Result<Vec<GeneratedImage>, ProviderError> {
            Ok(self.images.clone())
        }
    }

    // 1x1 transparent PNG.
    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    fn image_runtime() -> (Arc<LlmRouter>, Arc<MemStorage>) {
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
            models: "img-model".into(),
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
            ("default".to_owned(), "img-model".to_owned()),
            std::sync::Arc::new(ModelInfo {
                name: "img-model".to_owned(),
                model_type: crate::llm::models::model::LlmModelType::Image,
                pricing: crate::llm::cache::Pricing {
                    price_mode: crate::llm::models::model::LlmPriceMode::PerCall,
                    input_price: 0.0,
                    output_price: 0.0,
                    cache_read_price: None,
                    cache_write_price: None,
                    call_price: Some(0.01),
                },
                params: None,
            }),
        );
        cache.rebuild_routes();
        let router = LlmRouter::from_cache_for_test(cache);
        router.providers.insert(
            (SnowflakeId(1), 0),
            Arc::new(MockImageProvider {
                images: vec![
                    GeneratedImage {
                        b64_json: Some(PNG_B64.into()),
                        url: None,
                    },
                    GeneratedImage {
                        b64_json: Some(PNG_B64.into()),
                        url: None,
                    },
                ],
            }) as Arc<dyn ModelProvider>,
        );
        (router, Arc::new(MemStorage::default()))
    }

    fn image_node(config: Value) -> GraphNode {
        GraphNode {
            id: "img1".into(),
            data: NodeData {
                kind: "image".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn generates_persists_and_references_assets() {
        let (router, storage) = image_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = image_node(json!({
            "model": "img-model",
            "prompt": "一只猫，风格：{{#start.style#}}",
            "n": 2
        }));
        let mut pool = Pool::new();
        pool.entry("start".into())
            .or_default()
            .insert("style".into(), json!("水墨"));

        let out = run_image(
            &runtime,
            &(storage.clone() as Arc<dyn Storage>),
            &node,
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(out.output["n"], 2);
        let images = out.output["images"].as_array().unwrap();
        assert_eq!(images.len(), 2);
        assert!(images[0]["key"].as_str().unwrap().starts_with("gen/flows/"));
        assert!(images[0]["key"].as_str().unwrap().ends_with(".png"));
        assert_eq!(out.usage.unwrap()["images"], 2);
        // bytes persisted and sniffed as png
        let key = images[0]["key"].as_str().unwrap();
        let bytes = storage.get(key).await.unwrap();
        assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
    }

    #[tokio::test]
    async fn missing_template_ref_is_bad_request() {
        let (router, storage) = image_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = image_node(json!({
            "model": "img-model", "prompt": "{{#start.nope#}}"
        }));
        let err = run_image(
            &runtime,
            &(storage as Arc<dyn Storage>),
            &node,
            &Pool::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }

    #[test]
    fn image_config_validation() {
        assert!(super::validate(&json!({"prompt": "a cat"})).is_ok());
        assert!(
            super::validate(&json!({"prompt": "  "})).is_err(),
            "空 prompt"
        );
        assert!(
            super::validate(&json!({"prompt": "x", "n": 0})).is_err(),
            "n<1"
        );
        assert!(
            super::validate(&json!({"prompt": "x", "n": 11})).is_err(),
            "n>10"
        );
        assert!(
            super::validate(&json!({"prompt": "x", "timeout_ms": 0})).is_err(),
            "timeout<1"
        );
    }
}

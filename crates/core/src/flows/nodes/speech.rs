//! `speech` node executor (media-nodes.md §3) — synchronous TTS through the
//! llm foundation facade (`LlmRouter::call().speech()`): routing, key pool,
//! failover, per-character billing and the `Tts` model-type gate all live in
//! the kernel. The audio bytes are persisted to storage and referenced by
//! `{key, url}`. ASR (transcribe) is deliberately not a node yet — a real
//! use case (e.g. subtitle alignment) triggers it (roadmap discipline).

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

use super::LlmRuntime;

/// `speech` node config (media-nodes.md §3.1). `text` is a C3.1 template —
/// narration copy usually arrives from an upstream `chat` node's output.
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SpeechConfig {
    /// TTS model; absent → tenant option `llm.default_speech_model`.
    pub model: Option<String>,
    /// Narration text template (`{{#ns.field#}}` refs allowed).
    pub text: String,
    /// Upstream voice wire value (e.g. `alloy`, `zh-CN-XiaoxiaoNeural`).
    pub voice: String,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_ms: Option<i64>,
}

/// Config validation (media-nodes.md §3.1 bounds).
pub(super) fn validate(config: &Value) -> AppResult<()> {
    let c: SpeechConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'speech' config invalid: {e}")))?;
    if c.text.trim().is_empty() {
        return Err(AppError::BadRequest("speech: text 不能为空".into()));
    }
    if c.voice.trim().is_empty() {
        return Err(AppError::BadRequest("speech: voice 不能为空".into()));
    }
    if c.timeout_ms.is_some_and(|t| t < 1) {
        return Err(AppError::BadRequest(
            "speech: timeout_ms 须为 ≥1 的整数".into(),
        ));
    }
    Ok(())
}

/// Execute the `speech` node against the variable pool.
///
/// # Errors
/// `BadRequest` on missing template refs or a non-retryable provider error;
/// `Internal` on transient provider/timeout/storage failures.
pub async fn run_speech(
    runtime: &LlmRuntime,
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: SpeechConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("speech config: {e}")))?;
    let text = super::render_prompt_text(&cfg.text, pool)?;
    let voice = cfg.voice.trim().to_string();
    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(120_000) as u64;
    let started = Instant::now();
    let chars = text.chars().count();

    let call = runtime
        .router
        .call(&runtime.tenant, crate::llm::models::log::LogSource::Flow);
    let bytes = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        call.speech(cfg.model.as_deref(), &text, &voice),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("speech 超时 {timeout_ms}ms")))??;
    if bytes.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "speech: 上游返回空音频"
        )));
    }

    let run_id = crate::utils::id::new_id().to_string();
    let key = format!("gen/flows/{run_id}/speech.mp3");
    storage.put(&key, &bytes, "audio/mpeg").await?;
    let public_url = storage
        .url(&key)
        .await
        .unwrap_or_else(|_| format!("/{key}"));

    let latency_ms = started.elapsed().as_millis() as i64;
    let mut out = Map::new();
    out.insert("audio".into(), json!({ "key": key, "url": public_url }));
    out.insert("chars".into(), json!(chars));
    out.insert("voice".into(), json!(voice));
    out.insert("model".into(), json!(cfg.model));
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: Some(json!({ "chars": chars })),
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
    use raisfast_agent::provider::{ModelProvider, ProviderError};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

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

    struct MockSpeechProvider;
    impl MockSpeechProvider {
        fn heard() -> &'static Mutex<Vec<(String, String, String)>> {
            static HEARD: std::sync::OnceLock<Mutex<Vec<(String, String, String)>>> =
                std::sync::OnceLock::new();
            HEARD.get_or_init(|| Mutex::new(Vec::new()))
        }
    }

    #[async_trait]
    impl ModelProvider for MockSpeechProvider {
        fn name(&self) -> &str {
            "mock-speech"
        }
        async fn chat(
            &self,
            _request: &raisfast_agent::provider::ChatRequest<'_>,
            _model: &str,
        ) -> Result<raisfast_agent::ChatResponse, ProviderError> {
            Err(ProviderError::Transport("mock: chat unsupported".into()))
        }
        async fn speech(
            &self,
            text: &str,
            voice: &str,
            model: &str,
        ) -> Result<Vec<u8>, ProviderError> {
            Self::heard()
                .lock()
                .unwrap()
                .push((text.into(), voice.into(), model.into()));
            Ok(b"ID3\x03fake-mp3-bytes".to_vec())
        }
    }

    fn speech_runtime() -> (Arc<LlmRouter>, Arc<MemStorage>) {
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
            models: "tts-model".into(),
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
            ("default".to_owned(), "tts-model".to_owned()),
            std::sync::Arc::new(ModelInfo {
                name: "tts-model".to_owned(),
                model_type: crate::llm::models::model::LlmModelType::Tts,
                pricing: crate::llm::cache::Pricing {
                    price_mode: crate::llm::models::model::LlmPriceMode::Token,
                    input_price: 1.0,
                    output_price: 0.0,
                    cache_read_price: None,
                    cache_write_price: None,
                    call_price: None,
                },
                params: None,
            }),
        );
        cache.rebuild_routes();
        let router = LlmRouter::from_cache_for_test(cache);
        router.providers.insert(
            (SnowflakeId(1), 0),
            Arc::new(MockSpeechProvider) as Arc<dyn ModelProvider>,
        );
        (router, Arc::new(MemStorage::default()))
    }

    fn speech_node(config: Value) -> GraphNode {
        GraphNode {
            id: "tts1".into(),
            data: NodeData {
                kind: "speech".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn synthesizes_persists_and_reports_chars() {
        let (router, storage) = speech_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = speech_node(json!({
            "model": "tts-model",
            "text": "旁白：{{#chat_1.text#}}",
            "voice": "alloy"
        }));
        let mut pool = Pool::new();
        pool.entry("chat_1".into())
            .or_default()
            .insert("text".into(), json!("第一集开场"));

        let out = run_speech(
            &runtime,
            &(storage.clone() as Arc<dyn Storage>),
            &node,
            &pool,
        )
        .await
        .unwrap();
        let key = out.output["audio"]["key"].as_str().unwrap();
        assert!(key.starts_with("gen/flows/") && key.ends_with("/speech.mp3"));
        let bytes = storage.get(key).await.unwrap();
        assert!(&bytes[..3] == b"ID3");
        assert_eq!(out.output["chars"], "旁白：第一集开场".chars().count());
        assert_eq!(out.output["voice"], "alloy");
        assert!(out.usage.unwrap()["chars"].as_i64().unwrap() >= 1);
        let heard = MockSpeechProvider::heard().lock().unwrap();
        assert_eq!(heard[0].0, "旁白：第一集开场");
        assert_eq!(heard[0].1, "alloy");
    }

    #[test]
    fn speech_config_validation() {
        assert!(super::validate(&json!({"text": "hi", "voice": "alloy"})).is_ok());
        assert!(
            super::validate(&json!({"voice": "alloy"})).is_err(),
            "缺 text"
        );
        assert!(super::validate(&json!({"text": "hi"})).is_err(), "缺 voice");
        assert!(
            super::validate(&json!({"text": "hi", "voice": " ", "timeout_ms": 5})).is_err(),
            "空 voice"
        );
    }
}

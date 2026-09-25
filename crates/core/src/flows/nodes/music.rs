//! `music` 节点 executor（media-nodes §7 P1-8）：文生音乐，经 llm 底座
//! facade `music()`（路由/号池/failover/计费/类型守门全在内核）。产物转存
//! storage 并以 `{key, url}` 出口——典型消费方是 render 节点的 BGM 输入。
//!
//! 厂商现状：MiniMax music-01/1.5（同步返回，hex 音频）。同步直通节点，
//! 不涉及挂起/轮询。

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Map, Value, json};

use raisfast_agent::provider::MusicRequest;

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

use super::LlmRuntime;

/// `music` 节点 config。`model` 必填（音乐价差大，不设租户默认）；
/// `prompt` 是风格/情绪描述模板；`lyrics` 可选歌词模板（music-01 类）。
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MusicConfig {
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub lyrics: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_ms: Option<i64>,
}

/// Config validation.
pub(super) fn validate(config: &Value) -> AppResult<()> {
    let c: MusicConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'music' config invalid: {e}")))?;
    if c.model.trim().is_empty() {
        return Err(AppError::BadRequest("music: model 必填".into()));
    }
    if c.prompt.trim().is_empty() {
        return Err(AppError::BadRequest("music: prompt 不能为空".into()));
    }
    if c.timeout_ms.is_some_and(|t| t < 1) {
        return Err(AppError::BadRequest(
            "music: timeout_ms 须为 ≥1 的整数".into(),
        ));
    }
    Ok(())
}

/// Execute the `music` node against the variable pool.
///
/// # Errors
/// `BadRequest` on missing template refs or a non-retryable provider error;
/// `Internal` on transient provider/timeout/storage failures.
pub async fn run_music(
    runtime: &LlmRuntime,
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: MusicConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("music config: {e}")))?;
    let prompt = super::render_prompt_text(&cfg.prompt, pool)?;
    let lyrics = match &cfg.lyrics {
        Some(l) if !l.trim().is_empty() => Some(super::render_prompt_text(l, pool)?),
        _ => None,
    };
    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(300_000) as u64;
    let started = Instant::now();

    let call = runtime
        .router
        .call(&runtime.tenant, crate::llm::models::log::LogSource::Flow);
    let request = MusicRequest { prompt, lyrics };
    let bytes = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        call.music(&cfg.model, &request),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("music 超时 {timeout_ms}ms")))??;
    if bytes.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!("music: 上游返回空音频")));
    }

    let run_id = crate::utils::id::new_id().to_string();
    let key = format!("gen/flows/{run_id}/music.mp3");
    storage.put(&key, &bytes, "audio/mpeg").await?;
    let public_url = storage
        .url(&key)
        .await
        .unwrap_or_else(|_| format!("/{key}"));

    let latency_ms = started.elapsed().as_millis() as i64;
    let mut out = Map::new();
    out.insert("audio".into(), json!({ "key": key, "url": public_url }));
    out.insert("model".into(), json!(cfg.model));
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: Some(json!({ "bytes": bytes.len() })),
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

    struct MockMusicProvider;
    impl MockMusicProvider {
        fn heard() -> &'static Mutex<Vec<(String, String, String)>> {
            static HEARD: std::sync::OnceLock<Mutex<Vec<(String, String, String)>>> =
                std::sync::OnceLock::new();
            HEARD.get_or_init(|| Mutex::new(Vec::new()))
        }
    }

    #[async_trait]
    impl ModelProvider for MockMusicProvider {
        fn name(&self) -> &str {
            "mock-music"
        }
        async fn chat(
            &self,
            _request: &raisfast_agent::provider::ChatRequest<'_>,
            _model: &str,
        ) -> Result<raisfast_agent::ChatResponse, ProviderError> {
            Err(ProviderError::Transport("mock: chat unsupported".into()))
        }
        async fn music(
            &self,
            request: &MusicRequest,
            model: &str,
        ) -> Result<Vec<u8>, ProviderError> {
            Self::heard().lock().unwrap().push((
                request.prompt.clone(),
                request.lyrics.clone().unwrap_or_default(),
                model.into(),
            ));
            Ok(b"ID3fake-music".to_vec())
        }
    }

    fn music_runtime() -> (Arc<LlmRouter>, Arc<MemStorage>) {
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
            models: "music-model".into(),
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
            ("default".to_owned(), "music-model".to_owned()),
            std::sync::Arc::new(ModelInfo {
                name: "music-model".to_owned(),
                model_type: crate::llm::models::model::LlmModelType::Music,
                pricing: crate::llm::cache::Pricing {
                    price_mode: crate::llm::models::model::LlmPriceMode::PerCall,
                    input_price: 0.0,
                    output_price: 0.0,
                    cache_read_price: None,
                    cache_write_price: None,
                    call_price: Some(0.3),
                },
                params: None,
            }),
        );
        cache.rebuild_routes();
        let router = LlmRouter::from_cache_for_test(cache);
        router.providers.insert(
            (SnowflakeId(1), 0),
            Arc::new(MockMusicProvider) as Arc<dyn ModelProvider>,
        );
        (router, Arc::new(MemStorage::default()))
    }

    fn music_node(config: Value) -> GraphNode {
        GraphNode {
            id: "bgm1".into(),
            data: NodeData {
                kind: "music".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn generates_persists_and_carries_prompt_lyrics() {
        let (router, storage) = music_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = music_node(json!({
            "model": "music-model",
            "prompt": "古风bgm，{{#start.mood#}}",
            "lyrics": "[verse]\n{{#chat_1.text#}}"
        }));
        let mut pool = Pool::new();
        pool.entry("start".into())
            .or_default()
            .insert("mood".into(), json!("宁静"));
        pool.entry("chat_1".into())
            .or_default()
            .insert("text".into(), json!("第一句歌词"));

        let out = run_music(
            &runtime,
            &(storage.clone() as Arc<dyn Storage>),
            &node,
            &pool,
        )
        .await
        .unwrap();
        let key = out.output["audio"]["key"].as_str().unwrap();
        assert!(key.starts_with("gen/flows/") && key.ends_with("/music.mp3"));
        let bytes = storage.get(key).await.unwrap();
        assert_eq!(&bytes[..3], b"ID3");
        assert_eq!(out.output["model"], "music-model");

        let heard = MockMusicProvider::heard().lock().unwrap();
        assert_eq!(heard[0].0, "古风bgm，宁静");
        assert_eq!(heard[0].1, "[verse]\n第一句歌词");
        assert_eq!(heard[0].2, "music-model");
    }

    #[tokio::test]
    async fn missing_template_ref_is_bad_request() {
        let (router, storage) = music_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = music_node(json!({
            "model": "music-model", "prompt": "{{#start.nope#}}"
        }));
        let err = run_music(
            &runtime,
            &(storage as Arc<dyn Storage>),
            &node,
            &Pool::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
    }

    #[test]
    fn music_config_validation() {
        assert!(super::validate(&json!({"model": "m", "prompt": "epic"})).is_ok());
        assert!(
            super::validate(&json!({"prompt": "x"})).is_err(),
            "缺 model"
        );
        assert!(
            super::validate(&json!({"model": "m"})).is_err(),
            "缺 prompt"
        );
        assert!(
            super::validate(&json!({"model": "m", "prompt": "x", "timeout_ms": 0})).is_err(),
            "timeout<1"
        );
    }
}

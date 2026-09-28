//! `transcribe` 节点 executor（media-platform-roadmap.md P1-6）：语音转文字
//! （ASR），经 llm 底座 facade `transcribe()`（路由/号池/failover/按时长计费/
//! `Asr` 类型守门全在内核）。产物：纯文本 + 带时间戳分段；分段存在时额外
//! 生成 SRT 字幕文件转存 storage 并以 `{key, url}` 出口——`srt` 可直接喂
//! render 节点的 `subtitles` 输入（materialize 保留扩展名 → `sub.srt`，
//! 与 ffmpeg `subtitles=sub.srt` 烧录约定对齐）。
//!
//! 厂商现状：OpenAI-compat `/audio/transcriptions`（whisper 系通用协议，
//! OpenAI/Groq/SiliconFlow/本地 faster-whisper-server 均适用）。同步直通
//! 节点，不涉及挂起/轮询。

use std::sync::Arc;
use std::time::Instant;

use raisfast_agent::provider::{AudioInput, TranscriptSegment};
use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

use super::LlmRuntime;

/// `transcribe` 节点 config。`model` 必填（facade 无租户默认 ASR 模型）；
/// `audio` 是 ValueExpr，解析为 storage key / https URL / `{key|url}` 对象
/// （speech/speech 产物、start 文件参数、上游 music 节点的 `audio` 均可直连）。
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TranscribeConfig {
    pub model: String,
    /// 音频来源（ValueExpr → 字符串 / `{key|url}` 对象）。
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub audio: Value,
    /// BCP-47 语言提示（缺省 = 厂商自动检测）。
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_ms: Option<i64>,
}

/// Config validation.
pub(super) fn validate(config: &Value) -> AppResult<()> {
    let c: TranscribeConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'transcribe' config invalid: {e}")))?;
    if c.model.trim().is_empty() {
        return Err(AppError::BadRequest("transcribe: model 必填".into()));
    }
    super::validate_value_expr("transcribe", &c.audio)?;
    if c.language.as_deref().is_some_and(|l| l.trim().is_empty()) {
        return Err(AppError::BadRequest(
            "transcribe: language 不能为空字符串".into(),
        ));
    }
    if c.timeout_ms.is_some_and(|t| t < 1) {
        return Err(AppError::BadRequest(
            "transcribe: timeout_ms 须为 ≥1 的整数".into(),
        ));
    }
    Ok(())
}

/// Format seconds as SRT timestamp `HH:MM:SS,mmm`.
fn fmt_srt_ts(secs: f64) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as i64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        total_ms / 3_600_000,
        (total_ms % 3_600_000) / 60_000,
        (total_ms % 60_000) / 1000,
        total_ms % 1000
    )
}

/// Render timed segments as an SRT document (SubRip).
#[must_use]
pub fn segments_to_srt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for (i, seg) in segments.iter().enumerate() {
        out.push_str(&(i + 1).to_string());
        out.push('\n');
        out.push_str(&fmt_srt_ts(seg.start));
        out.push_str(" --> ");
        out.push_str(&fmt_srt_ts(seg.end));
        out.push('\n');
        out.push_str(seg.text.trim());
        out.push_str("\n\n");
    }
    out
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

fn last_path_segment(reference: &str) -> String {
    reference
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Mime from the filename extension (the whisper wire mainly keys off the
/// extension; None → provider fallback).
fn mime_for_filename(filename: &str) -> Option<&'static str> {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "m4a" | "mp4" => Some("audio/mp4"),
        "ogg" | "oga" | "opus" => Some("audio/ogg"),
        "flac" => Some("audio/flac"),
        "webm" => Some("audio/webm"),
        _ => None,
    }
}

/// Resolve the `audio` ValueExpr and fetch the bytes: string → storage key or
/// https URL (SSRF-checked download); `{key|url}` object → either field.
async fn fetch_audio(storage: &Arc<dyn Storage>, resolved: &Value) -> AppResult<(Vec<u8>, String)> {
    let reference = match resolved {
        Value::String(s) => s.trim().to_string(),
        Value::Object(o) => o
            .get("key")
            .and_then(Value::as_str)
            .or_else(|| o.get("url").and_then(Value::as_str))
            .map(str::to_string)
            .ok_or_else(|| AppError::BadRequest("transcribe: audio 对象须含 key 或 url".into()))?,
        other => {
            return Err(AppError::BadRequest(format!(
                "transcribe: audio 须为字符串或 {{key|url}} 对象（got {}）",
                json_typename(other)
            )));
        }
    };
    if reference.is_empty() {
        return Err(AppError::BadRequest("transcribe: audio 不能为空".into()));
    }
    let (bytes, filename) = if reference.starts_with("https://") || reference.starts_with("http://")
    {
        (
            super::download_https(&reference).await?,
            last_path_segment(&reference),
        )
    } else {
        let bytes = storage.get(&reference).await.map_err(|e| {
            AppError::BadRequest(format!("transcribe: audio 读取 {reference} 失败: {e}"))
        })?;
        (bytes, last_path_segment(&reference))
    };
    let filename = if filename.is_empty() {
        "audio.mp3".to_owned()
    } else {
        filename
    };
    Ok((bytes, filename))
}

/// Execute the `transcribe` node against the variable pool.
///
/// # Errors
/// `BadRequest` on a malformed audio reference or a non-retryable provider
/// error; `Internal` on transient provider/timeout/storage failures.
pub async fn run_transcribe(
    runtime: &LlmRuntime,
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: TranscribeConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("transcribe config: {e}")))?;
    let resolved = crate::flows::engine::resolve(&cfg.audio, pool)?;
    let (bytes, filename) = fetch_audio(storage, &resolved).await?;
    let filename_for_mime = filename.clone();
    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(300_000) as u64;
    let started = Instant::now();

    let call = runtime
        .router
        .call(&runtime.tenant, crate::llm::models::log::LogSource::Flow);
    let audio = AudioInput {
        data: &bytes,
        filename,
        mime: mime_for_filename(&filename_for_mime).map(str::to_owned),
        language: cfg.language.clone(),
    };
    let transcription = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        call.transcribe(&cfg.model, &audio),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("transcribe 超时 {timeout_ms}ms")))??;

    let segments_json: Vec<Value> = transcription
        .segments
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "start": s.start,
                "end": s.end,
                "text": s.text,
            })
        })
        .collect();

    let latency_ms = started.elapsed().as_millis() as i64;
    let mut out = Map::new();
    out.insert("text".into(), json!(transcription.text));
    out.insert("segments".into(), Value::Array(segments_json));
    out.insert("model".into(), json!(cfg.model));
    if !transcription.segments.is_empty() {
        let srt = segments_to_srt(&transcription.segments);
        let run_id = crate::utils::id::new_id().to_string();
        let key = format!("gen/flows/{run_id}/sub.srt");
        storage
            .put(&key, srt.as_bytes(), "application/x-subrip")
            .await?;
        let public_url = storage
            .url(&key)
            .await
            .unwrap_or_else(|_| format!("/{key}"));
        out.insert("srt".into(), json!({ "key": key, "url": public_url }));
        let duration = transcription
            .segments
            .iter()
            .map(|s| s.end)
            .fold(0.0_f64, f64::max);
        out.insert("duration".into(), json!(duration));
    }
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
    use raisfast_agent::provider::{ModelProvider, ProviderError, Transcription};
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

    struct MockAsrProvider;
    type HeardEntry = (String, String, Option<String>);
    impl MockAsrProvider {
        fn heard() -> &'static Mutex<Vec<HeardEntry>> {
            static HEARD: std::sync::OnceLock<Mutex<Vec<HeardEntry>>> = std::sync::OnceLock::new();
            HEARD.get_or_init(|| Mutex::new(Vec::new()))
        }
    }

    #[async_trait]
    impl ModelProvider for MockAsrProvider {
        fn name(&self) -> &str {
            "mock-asr"
        }
        async fn chat(
            &self,
            _request: &raisfast_agent::provider::ChatRequest<'_>,
            _model: &str,
        ) -> Result<raisfast_agent::ChatResponse, ProviderError> {
            Err(ProviderError::Transport("mock: chat unsupported".into()))
        }
        async fn transcribe(
            &self,
            audio: &AudioInput<'_>,
            model: &str,
        ) -> Result<Transcription, ProviderError> {
            Self::heard().lock().unwrap().push((
                audio.filename.clone(),
                model.to_owned(),
                audio.language.clone(),
            ));
            let text = if audio.filename.ends_with(".mp3") {
                " 你好世界。 这是第二段。"
            } else {
                "纯文本无分段"
            };
            let segments = if audio.filename.ends_with(".mp3") {
                vec![
                    TranscriptSegment {
                        id: 0,
                        start: 0.0,
                        end: 2.5,
                        text: " 你好世界。".into(),
                    },
                    TranscriptSegment {
                        id: 1,
                        start: 2.5,
                        end: 6.08,
                        text: " 这是第二段。".into(),
                    },
                ]
            } else {
                Vec::new()
            };
            Ok(Transcription {
                text: text.into(),
                segments,
            })
        }
    }

    fn asr_runtime() -> (Arc<LlmRouter>, Arc<MemStorage>) {
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
            models: "asr-model".into(),
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
            ("default".to_owned(), "asr-model".to_owned()),
            std::sync::Arc::new(ModelInfo {
                name: "asr-model".to_owned(),
                model_type: crate::llm::models::model::LlmModelType::Asr,
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
            Arc::new(MockAsrProvider) as Arc<dyn ModelProvider>,
        );
        (router, Arc::new(MemStorage::default()))
    }

    fn transcribe_node(config: Value) -> GraphNode {
        GraphNode {
            id: "asr1".into(),
            data: NodeData {
                kind: "transcribe".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn transcribes_and_emits_srt() {
        let (router, storage) = asr_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let audio_key = "uploads/voice-over.mp3";
        crate::storage::Storage::put(storage.as_ref(), audio_key, b"ID3fake-mp3", "audio/mpeg")
            .await
            .unwrap();
        let node = transcribe_node(json!({
            "model": "asr-model",
            "audio": {"ref": ["start", "narration"]},
            "language": "zh"
        }));
        let mut pool = Pool::new();
        pool.entry("start".into()).or_default().insert(
            "narration".into(),
            json!({ "key": audio_key, "url": format!("http://localhost/{audio_key}") }),
        );

        let out = run_transcribe(
            &runtime,
            &(storage.clone() as Arc<dyn Storage>),
            &node,
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(out.output["text"], " 你好世界。 这是第二段。");
        assert_eq!(out.output["segments"].as_array().unwrap().len(), 2);
        assert_eq!(out.output["model"], "asr-model");
        assert_eq!(out.output["duration"], 6.08);
        let srt_key = out.output["srt"]["key"].as_str().unwrap();
        assert!(srt_key.starts_with("gen/flows/") && srt_key.ends_with("/sub.srt"));
        let srt = storage.get(srt_key).await.unwrap();
        let srt = String::from_utf8(srt).unwrap();
        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:02,500\n你好世界。\n\n2\n00:00:02,500 --> 00:00:06,080\n这是第二段。\n\n"
        );
        assert!(out.usage.unwrap()["bytes"].as_i64().unwrap() >= 1);
        // Scoped: parallel tests share the global heard registry — match by
        // content instead of assuming ordering.
        let heard = MockAsrProvider::heard().lock().unwrap();
        let call = heard
            .iter()
            .find(|(f, m, _)| f == "voice-over.mp3" && m == "asr-model")
            .expect("asr call recorded");
        assert_eq!(call.2.as_deref(), Some("zh"));
    }

    #[tokio::test]
    async fn no_segments_omits_srt_field() {
        let (router, storage) = asr_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let audio_key = "uploads/notes.txt";
        crate::storage::Storage::put(storage.as_ref(), audio_key, b"xx", "text/plain")
            .await
            .unwrap();
        // String literal shorthand (storage key) — no timed segments upstream.
        let node = transcribe_node(json!({
            "model": "asr-model",
            "audio": {"literal": audio_key}
        }));
        let out = run_transcribe(
            &runtime,
            &(storage.clone() as Arc<dyn Storage>),
            &node,
            &Pool::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.output["text"], "纯文本无分段");
        assert!(out.output.get("srt").is_none(), "无分段不出 SRT");
        assert!(out.output.get("duration").is_none());
    }

    #[tokio::test]
    async fn missing_audio_ref_is_bad_request() {
        let (router, storage) = asr_runtime();
        let runtime = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = transcribe_node(json!({
            "model": "asr-model",
            "audio": {"ref": ["start", "nope"]}
        }));
        let err = run_transcribe(
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
    fn transcribe_config_validation() {
        assert!(
            super::validate(&json!({"model": "m", "audio": {"ref": ["a", "b"]}})).is_ok(),
            "最小配置"
        );
        assert!(
            super::validate(&json!({"audio": {"ref": ["a", "b"]}})).is_err(),
            "缺 model"
        );
        assert!(
            super::validate(&json!({"model": "m", "audio": {"ref": "a"}})).is_err(),
            "ref 非字符串数组"
        );
        assert!(
            super::validate(&json!({"model": "m", "audio": {"ref": ["a"]}, "timeout_ms": 0}))
                .is_err(),
            "timeout<1"
        );
        assert!(
            super::validate(&json!({"model": "m", "audio": {"ref": ["a"]}, "language": " "}))
                .is_err(),
            "空 language"
        );
    }

    #[test]
    fn srt_timestamp_formatting() {
        assert_eq!(fmt_srt_ts(0.0), "00:00:00,000");
        assert_eq!(fmt_srt_ts(2.5), "00:00:02,500");
        assert_eq!(fmt_srt_ts(6.08), "00:00:06,080");
        assert_eq!(fmt_srt_ts(3_723.456), "01:02:03,456");
    }
}

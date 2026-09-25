//! ElevenLabs provider for internal consumption: implements the agent
//! `ModelProvider` speech (TTS) surface over the native
//! `POST /text-to-speech/{voice_id}` protocol, so flows can route
//! `provider: "elevenlabs"` channels onto `LlmRouter::call().speech()`.
//!
//! Reference matrix:
//! - provider-for-internal-consumption shape: [照抄本仓
//!   `llm/providers/anthropic.rs`] — same constructor surface
//!   (base_url/api_key/param_override/header_override), same "non-native
//!   modalities fall back to trait defaults (unsupported) so the kernel
//!   skips the channel" discipline.
//! - auth header: ElevenLabs uses `xi-api-key` (NOT Bearer) — [参考
//!   ElevenLabs 公开 API 文档，本地无 third 源码].
//! - response is raw audio bytes (no JSON envelope, no usage): billing is
//!   input-side character estimate [照抄本仓 facade `speech()` 预扣估算].
//! - errors arrive as `{detail: {message, status}}` — collapsed into a
//!   one-line `Http` body so kernel failover classification stays readable
//!   [照抄 providers/anthropic.rs 错误压平纪律].

use async_trait::async_trait;
use serde_json::{Value, json};

use raisfast_agent::provider::{ChatRequest, ChatResponse, ModelProvider, ProviderError};

use crate::llm::relay::adaptor::shared_client;

pub struct ElevenLabsProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    param_override: Option<Value>,
    header_override: Option<Value>,
}

/// Default TTS wire format when the channel carries no
/// `param_override.output_format` (ElevenLabs query param).
const DEFAULT_OUTPUT_FORMAT: &str = "mp3_44100_128";

impl ElevenLabsProvider {
    /// `base_url` is the ElevenLabs root including the version segment,
    /// e.g. `https://api.elevenlabs.io/v1`.
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

    fn speech_url(&self, voice_id: &str) -> String {
        let output_format = self
            .param_override
            .as_ref()
            .and_then(|o| o.get("output_format"))
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_OUTPUT_FORMAT);
        format!(
            "{}/text-to-speech/{}?output_format={}",
            self.base_url.trim_end_matches('/'),
            voice_id,
            output_format
        )
    }

    fn build_body(&self, text: &str, model: &str) -> Value {
        let mut body = json!({
            "text": text,
            "model_id": model,
            // ElevenLabs-blessed defaults [照抄 MPT voice.py:elevenlabs_tts] —
            // omitting voice_settings lets the upstream pick, which drifts
            // voice character between renders.
            "voice_settings": {
                "stability": 0.5,
                "similarity_boost": 0.75,
                "style": 0.0,
                "use_speaker_boost": true,
            },
        });
        if let Some(settings) = self
            .param_override
            .as_ref()
            .and_then(|o| o.get("voice_settings"))
        {
            body["voice_settings"] = settings.clone();
        }
        body
    }

    async fn send(&self, url: &str, body: &Value) -> Result<Vec<u8>, ProviderError> {
        let mut req = self.http.post(url).json(body);
        if let Some(key) = &self.api_key {
            req = req.header("xi-api-key", key);
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
        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            // Error envelope: {detail: {message, status}} — compact it so the
            // kernel's failover classification and llm_logs stay readable.
            let body_text = String::from_utf8_lossy(&bytes);
            let message = serde_json::from_str::<Value>(&body_text)
                .ok()
                .and_then(|v| {
                    v.get("detail")
                        .and_then(|d| d.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body_text.lines().next().unwrap_or_default().to_string());
            return Err(ProviderError::Http {
                status,
                body: message,
            });
        }
        if bytes.is_empty() {
            return Err(ProviderError::Parse(format!(
                "elevenlabs: empty audio response (content-type {content_type})"
            )));
        }
        Ok(bytes.to_vec())
    }
}

#[async_trait]
impl ModelProvider for ElevenLabsProvider {
    fn name(&self) -> &str {
        "elevenlabs"
    }

    /// Required by the trait; ElevenLabs has no chat surface — return
    /// `Config` so the kernel skips (not fails) this channel.
    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Config(
            "provider elevenlabs does not support chat".into(),
        ))
    }

    /// 文生语音：`POST {base}/text-to-speech/{voice_id}`，同步返回音频字节。
    /// 计费为 input-side 字符估算（facade `speech()` 语义），无 usage 回传。
    async fn speech(&self, text: &str, voice: &str, model: &str) -> Result<Vec<u8>, ProviderError> {
        let url = self.speech_url(voice);
        let body = self.build_body(text, model);
        self.send(&url, &body).await
    }

    // Non-speech modalities are intentionally NOT overridden: ElevenLabs has
    // no chat/embedding/image/video APIs on this surface, so the trait
    // defaults ("does not support") stand and the kernel skips these
    // channels for such calls (failover, no failure report).
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// base_url carries the version segment (preset default
    /// `https://api.elevenlabs.io/v1`), mirrored here with `/v1`.
    fn provider(base_url: String) -> ElevenLabsProvider {
        ElevenLabsProvider::new(format!("{base_url}/v1"), Some("sk-test".into()), None, None)
    }

    #[tokio::test]
    async fn speech_posts_to_voice_path_with_xi_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/alloy"))
            .and(header("xi-api-key", "sk-test"))
            .and(query_param("output_format", "mp3_44100_128"))
            .and(body_partial_json(json!({"text": "你好旁白"})))
            .and(body_partial_json(
                json!({"model_id": "eleven_multilingual_v2"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(b"ID3fake-mp3".to_vec()),
            )
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let bytes = p
            .speech("你好旁白", "alloy", "eleven_multilingual_v2")
            .await
            .unwrap();
        assert_eq!(&bytes[..3], b"ID3");
    }

    #[tokio::test]
    async fn default_voice_settings_and_override() {
        // Defaults [照抄 MPT elevenlabs_tts].
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"voice_settings": {"stability": 0.5, "similarity_boost": 0.75, "use_speaker_boost": true}})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(b"ID3".to_vec()),
            )
            .mount(&server)
            .await;
        let p = provider(server.uri());
        p.speech("x", "alloy", "m").await.unwrap();

        // Channel param_override replaces the whole voice_settings object.
        let server2 = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({"voice_settings": {"stability": 0.9}}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(b"ID3".to_vec()),
            )
            .mount(&server2)
            .await;
        let p2 = ElevenLabsProvider::new(
            server2.uri(),
            Some("sk-test".into()),
            Some(json!({"voice_settings": {"stability": 0.9}})),
            None,
        );
        p2.speech("x", "alloy", "m").await.unwrap();
    }

    #[tokio::test]
    async fn error_envelope_compacts_to_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(
                json!({"detail": {"message": "invalid api key", "status": "unauthorized"}}),
            ))
            .mount(&server)
            .await;

        let p = provider(server.uri());
        let err = p.speech("x", "alloy", "m").await.unwrap_err();
        match err {
            ProviderError::Http { status, body } => {
                assert_eq!(status, 401);
                assert_eq!(body, "invalid api key");
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }
}

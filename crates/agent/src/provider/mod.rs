pub mod openai;

// Native-protocol providers that need host services (the anthropic codec,
// shared HTTP client, param-override semantics) live in the host crate:
// `core/src/llm/provider_anthropic.rs` — the dependency direction is
// core → agent, so they cannot sit here. This directory holds
// self-contained transports (URL + key is all they need).

use async_trait::async_trait;

use crate::messages::{ChatMessage, TokenUsage, ToolCall};
use crate::tool::ToolSpec;

/// A chat request to a provider.
#[derive(Debug)]
pub struct ChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [ToolSpec]>,
    pub temperature: Option<f64>,
    /// Sampling budget; provider wire name `max_tokens` (None = server default).
    pub max_tokens: Option<i64>,
    /// Up to 4 stop sequences (OpenAI wire limit).
    pub stop: Option<Vec<String>>,
}

/// A completed non-streaming chat response.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
}

impl ChatResponse {
    pub fn text_only(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            tool_calls: Vec::new(),
            usage: None,
        }
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// A live event while a response is being generated.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta {
        delta: String,
    },
    /// Reasoning/thinking delta (shown separately, never merged into text).
    ReasoningDelta {
        delta: String,
    },
    /// A fully assembled tool call (emitted at end of stream).
    ToolCall(ToolCall),
    Usage(TokenUsage),
    Final,
}

/// One rerank result: document position in the input plus relevance score.
#[derive(Debug, Clone)]
pub struct RerankResult {
    pub index: usize,
    pub relevance_score: f32,
}

/// A text-to-image request (OpenAI `/images/generations` semantics).
#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub prompt: String,
    /// Number of images to generate (wire default 1).
    pub n: u32,
    /// Wire size string, e.g. `1024x1024` (None = provider default).
    pub size: Option<String>,
}

/// One generated image — exactly one of the fields is set.
#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub b64_json: Option<String>,
    pub url: Option<String>,
}

/// Audio upload for transcription (`/audio/transcriptions` semantics).
#[derive(Debug, Clone)]
pub struct AudioInput<'a> {
    pub data: &'a [u8],
    pub filename: String,
    pub mime: Option<String>,
    /// BCP-47 hint (None = provider auto-detect).
    pub language: Option<String>,
}

/// Speech-to-text result.
#[derive(Debug, Clone)]
pub struct Transcription {
    pub text: String,
}

/// An async video-generation task (OpenAI Videos API semantics: submit
/// returns a task, content is fetched separately once completed).
#[derive(Debug, Clone)]
pub struct VideoRequest {
    pub prompt: String,
    /// Seconds of footage (string on the wire; None = provider default).
    pub seconds: Option<String>,
    /// Wire size string, e.g. `1280x720` (None = provider default).
    pub size: Option<String>,
}

/// Terminal-or-not state of an async video task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
}

impl VideoStatus {
    /// Map a wire status string (`queued`/`in_progress`/`completed`/
    /// `failed`/`expired`; unknown → `Failed` so consumers don't wait
    /// forever on a state they don't know).
    pub fn from_wire(s: &str) -> Self {
        match s {
            "queued" => Self::Queued,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            _ => Self::Failed,
        }
    }

    /// Whether polling can stop.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// Snapshot of an async video task.
#[derive(Debug, Clone)]
pub struct VideoTask {
    pub id: String,
    pub status: VideoStatus,
    /// Provider progress 0-100 when reported.
    pub progress: Option<i32>,
    /// Failure reason when `status == Failed`.
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider config error: {0}")]
    Config(String),
    #[error("http {status}: {body}")]
    Http { status: u16, body: String },
    #[error("transport error: {0}")]
    Transport(String),
    #[error("cannot parse provider response: {0}")]
    Parse(String),
}

/// Abstraction over an LLM chat provider (non-streaming for MVP).
#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn chat(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
    ) -> Result<ChatResponse, ProviderError>;

    /// Text embeddings via the OpenAI-compatible `POST {base}/embeddings`
    /// wire protocol (`{"model","input":[...]}` →
    /// `{"data":[{"embedding":[...],"index"}]}`). Implementations must
    /// return one vector per input text, in input order.
    async fn embed(&self, _texts: &[&str], _model: &str) -> Result<Vec<Vec<f32>>, ProviderError> {
        Err(ProviderError::Config(format!(
            "provider {} does not support embeddings",
            self.name()
        )))
    }

    /// Streaming variant: feed incremental events to `on_event` while the
    /// response is produced, and return the fully assembled response.
    /// Default = non-streaming `chat` replayed as a single batch of events.
    async fn chat_stream(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        let response = self.chat(request, model).await?;
        if let Some(text) = &response.text {
            on_event(StreamEvent::TextDelta {
                delta: text.clone(),
            });
        }
        for call in &response.tool_calls {
            on_event(StreamEvent::ToolCall(call.clone()));
        }
        if let Some(usage) = response.usage {
            on_event(StreamEvent::Usage(usage));
        }
        on_event(StreamEvent::Final);
        Ok(response)
    }

    // ── additional modalities (design §8: the six relay faces) ─────────
    //
    // Every method defaults to `ProviderError::Config` "does not support"
    // so the execution kernel treats the endpoint as unable to serve the
    // request and fails over to another channel without recording a failure
    // (mirrors the relay's provider guard).

    fn unsupported_message(&self, modality: &str) -> ProviderError {
        ProviderError::Config(format!(
            "provider {} does not support {modality}",
            self.name()
        ))
    }

    /// Relevance reranking (Jina/Cohere `/rerank` shape): score `documents`
    /// against `query`, best-first.
    async fn rerank(
        &self,
        _query: &str,
        _documents: &[&str],
        _model: &str,
    ) -> Result<Vec<RerankResult>, ProviderError> {
        Err(self.unsupported_message("rerank"))
    }

    /// Text-to-image (OpenAI `/images/generations` semantics).
    async fn generate_image(
        &self,
        _request: &ImageRequest,
        _model: &str,
    ) -> Result<Vec<GeneratedImage>, ProviderError> {
        Err(self.unsupported_message("image generation"))
    }

    /// Speech-to-text (`/audio/transcriptions` semantics).
    async fn transcribe(
        &self,
        _audio: &AudioInput<'_>,
        _model: &str,
    ) -> Result<Transcription, ProviderError> {
        Err(self.unsupported_message("transcription"))
    }

    /// Text-to-speech (`/audio/speech` semantics) — raw audio bytes.
    async fn speech(
        &self,
        _text: &str,
        _voice: &str,
        _model: &str,
    ) -> Result<Vec<u8>, ProviderError> {
        Err(self.unsupported_message("speech"))
    }

    /// Submit an async video-generation task (`POST /videos`).
    async fn video_submit(
        &self,
        _request: &VideoRequest,
        _model: &str,
    ) -> Result<VideoTask, ProviderError> {
        Err(self.unsupported_message("video generation"))
    }

    /// Poll an async video task (`GET /videos/{id}`).
    async fn video_query(&self, _task_id: &str, _model: &str) -> Result<VideoTask, ProviderError> {
        Err(self.unsupported_message("video generation"))
    }

    /// Fetch completed video bytes (`GET /videos/{id}/content`).
    async fn video_content(&self, _task_id: &str, _model: &str) -> Result<Vec<u8>, ProviderError> {
        Err(self.unsupported_message("video generation"))
    }
}

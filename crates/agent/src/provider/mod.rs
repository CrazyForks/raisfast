pub mod openai;

use async_trait::async_trait;

use crate::messages::{ChatMessage, TokenUsage, ToolCall};
use crate::tool::ToolSpec;

/// A chat request to a provider.
#[derive(Debug)]
pub struct ChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [ToolSpec]>,
    pub temperature: Option<f64>,
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
}

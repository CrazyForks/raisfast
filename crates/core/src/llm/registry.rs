//! Provider code registry — UI preset sugar only (design §13).
//!
//! Creating a channel does NOT validate `provider` against this list
//! (`generic` is the catch-all for custom deployments).

use serde::Serialize;

/// One provider preset for admin UI dropdowns.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderPreset {
    /// Registry key stored on `llm_channels.provider`.
    pub key: &'static str,
    pub display_name: &'static str,
    /// Pre-filled upstream base URL (empty = user must supply).
    pub default_base_url: &'static str,
    /// Whether the upstream requires an API key.
    pub requires_auth: bool,
}

const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        key: "openai",
        display_name: "OpenAI",
        default_base_url: "https://api.openai.com/v1",
        requires_auth: true,
    },
    ProviderPreset {
        key: "anthropic",
        display_name: "Anthropic",
        default_base_url: "https://api.anthropic.com",
        requires_auth: true,
    },
    ProviderPreset {
        key: "gemini",
        display_name: "Google Gemini",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        requires_auth: true,
    },
    ProviderPreset {
        key: "deepseek",
        display_name: "DeepSeek",
        default_base_url: "https://api.deepseek.com/v1",
        requires_auth: true,
    },
    ProviderPreset {
        key: "zhipu",
        display_name: "Zhipu (GLM)",
        default_base_url: "https://open.bigmodel.cn/api/paas/v4",
        requires_auth: true,
    },
    ProviderPreset {
        key: "moonshot",
        display_name: "Moonshot (Kimi)",
        default_base_url: "https://api.moonshot.cn/v1",
        requires_auth: true,
    },
    ProviderPreset {
        key: "ollama",
        display_name: "Ollama (local)",
        default_base_url: "http://localhost:11434/v1",
        requires_auth: false,
    },
    ProviderPreset {
        key: "seedance",
        display_name: "Seedance (火山方舟/即梦, video)",
        default_base_url: "https://ark.cn-beijing.volces.com/api/v3",
        requires_auth: true,
    },
    ProviderPreset {
        key: "minimax",
        display_name: "MiniMax (海螺, video)",
        default_base_url: "https://api.minimaxi.com/v1",
        requires_auth: true,
    },
    ProviderPreset {
        key: "replicate",
        display_name: "Replicate (video/multi)",
        default_base_url: "https://api.replicate.com/v1",
        requires_auth: true,
    },
    ProviderPreset {
        key: "kling",
        display_name: "Kling AI (可灵, video)",
        default_base_url: "https://api.klingai.com",
        requires_auth: true,
    },
    ProviderPreset {
        key: "elevenlabs",
        display_name: "ElevenLabs (TTS)",
        default_base_url: "https://api.elevenlabs.io/v1",
        requires_auth: true,
    },
    ProviderPreset {
        key: "siliconflow",
        display_name: "SiliconFlow",
        default_base_url: "https://api.siliconflow.cn/v1",
        requires_auth: true,
    },
    ProviderPreset {
        key: "generic",
        display_name: "Generic (OpenAI-compatible)",
        default_base_url: "",
        requires_auth: false,
    },
];

/// Full preset list (admin dropdown).
pub fn registry() -> &'static [ProviderPreset] {
    PRESETS
}

/// Find one preset by key.
pub fn find(key: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.key == key)
}

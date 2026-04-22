//! OAuth2 社交登录模块
//!
//! 提供 OAuth2 Authorization Code + PKCE 流程的完整实现。
//! 每个 Provider 实现 [`OAuthProvider`] trait，通过 [`OAuthProviderRegistry`] 管理。
//!
//! ## 目录结构
//!
//! - `oauth/mod.rs` — trait、registry、PKCE 工具（本文件）
//! - `oauth/github.rs` — GitHub OAuth2 Provider
//! - `oauth/google.rs` — Google OAuth2 Provider

pub mod github;
pub mod google;
pub mod wechat;

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::app_error::AppResult;

/// OAuth Provider 用户信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserInfo {
    /// Provider 侧的用户 ID
    pub provider_user_id: String,
    /// 用户邮箱
    pub email: Option<String>,
    /// 显示名称
    pub display_name: Option<String>,
    /// 头像 URL
    pub avatar_url: Option<String>,
    /// Provider 返回的原始 profile JSON
    pub raw_profile: serde_json::Value,
}

/// OAuth Token 交换响应
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    #[allow(dead_code)]
    pub token_type: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    #[allow(dead_code)]
    pub scope: Option<String>,
}

/// OAuth Provider trait
///
/// 每个 OAuth Provider 实现此 trait，提供授权 URL 构建、code 交换、用户信息获取。
#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Provider 标识（如 "github"）
    fn name(&self) -> &str;

    /// 构建授权 URL
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String;

    /// 用 authorization code + code_verifier 换 access_token
    async fn exchange_code(&self, code: &str, code_verifier: &str)
    -> AppResult<OAuthTokenResponse>;

    /// 用 access_token 获取用户信息
    async fn fetch_user_info(&self, access_token: &str) -> AppResult<OAuthUserInfo>;
}

/// OAuth Provider 注册表
#[derive(Default)]
pub struct OAuthProviderRegistry {
    providers: HashMap<String, Box<dyn OAuthProvider>>,
}

impl OAuthProviderRegistry {
    /// 创建空注册表
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// 注册 Provider
    pub fn register(&mut self, provider: Box<dyn OAuthProvider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    /// 获取指定 Provider
    pub fn get(&self, name: &str) -> Option<&dyn OAuthProvider> {
        self.providers.get(name).map(|p| p.as_ref())
    }

    /// 获取已注册的 Provider 名称列表
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

// ── PKCE 工具 ────────────────────────────────────────────────

/// 生成随机 code_verifier（43 字符，满足 43-128 要求）
pub fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .unwrap_or_else(|e| panic!("code_verifier generation failed: {e}"));
    URL_SAFE_NO_PAD.encode(bytes)
}

/// 从 code_verifier 生成 code_challenge（S256 方法）
pub fn generate_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// 生成随机 state 参数（32 字节 hex）
pub fn generate_state() -> String {
    crate::utils::id::random_hex(32)
}

// ── 测试 ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_register_and_get() {
        let mut reg = OAuthProviderRegistry::new();
        assert!(reg.get("github").is_none());

        reg.register(Box::new(github::GitHubProvider::new(
            "test_id".into(),
            "test_secret".into(),
        )));
        assert!(reg.get("github").is_some());
        assert_eq!(reg.provider_names(), vec!["github"]);
    }

    #[test]
    fn pkce_code_challenge_deterministic() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = generate_code_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn pkce_verifier_length() {
        let verifier = generate_code_verifier();
        assert!((43..=128).contains(&verifier.len()));
    }

    #[test]
    fn state_is_hex_64_chars() {
        let state = generate_state();
        assert_eq!(state.len(), 64);
        assert!(state.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn github_authorize_url_format() {
        let provider = github::GitHubProvider::new("my_client_id".into(), "secret".into());
        let url = provider.authorize_url("state123", "challenge456");
        assert!(url.contains("client_id=my_client_id"));
        assert!(url.contains("state=state123"));
        assert!(url.contains("code_challenge=challenge456"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=user:email"));
    }

    #[test]
    fn google_authorize_url_format() {
        let provider = google::GoogleProvider::new("my_client_id".into(), "secret".into());
        let url = provider.authorize_url("state123", "challenge456");
        assert!(url.contains("client_id=my_client_id"));
        assert!(url.contains("state=state123"));
        assert!(url.contains("scope=openid+email+profile"));
    }
}

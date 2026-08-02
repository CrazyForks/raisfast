//! OAuth2 social login module
//!
//! Provides a complete implementation of the OAuth2 Authorization Code + PKCE flow.
//! Each provider implements the [`OAuthProvider`] trait, managed via [`OAuthProviderRegistry`].
//!
//! ## Directory structure
//!
//! - `oauth/mod.rs` — trait, registry, PKCE utilities (this file)
//! - `oauth/github.rs` — GitHub OAuth2 Provider
//! - `oauth/google.rs` — Google OAuth2 Provider

pub mod github;
pub mod google;
pub mod wechat;

/// HTTP request timeout for OAuth provider calls (seconds)
const OAUTH_TIMEOUT_SECS: u64 = 10;

/// Build a reqwest client with OAuth-appropriate timeout
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(OAUTH_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::app_error::AppResult;

/// OAuth provider user info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserInfo {
    /// User ID on the provider side
    pub provider_user_id: String,
    /// User email
    pub email: Option<String>,
    /// Display name
    pub display_name: Option<String>,
    /// Avatar URL
    pub avatar_url: Option<String>,
    /// Raw profile JSON returned by the provider
    pub raw_profile: serde_json::Value,
}

/// OAuth token exchange response
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
/// Each OAuth provider implements this trait, providing authorize URL construction,
/// code exchange, and user info retrieval.
#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Provider identifier (e.g. "github")
    fn name(&self) -> &str;

    /// Build the authorize URL
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String;

    /// Exchange authorization code + code_verifier for an access_token
    async fn exchange_code(&self, code: &str, code_verifier: &str)
    -> AppResult<OAuthTokenResponse>;

    /// Fetch user info using an access_token
    async fn fetch_user_info(&self, access_token: &str) -> AppResult<OAuthUserInfo>;
}

/// OAuth Provider registry
#[derive(Default)]
pub struct OAuthProviderRegistry {
    providers: HashMap<String, Box<dyn OAuthProvider>>,
}

impl OAuthProviderRegistry {
    /// Create an empty registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a provider
    pub fn register(&mut self, provider: Box<dyn OAuthProvider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    /// Get a provider by name
    pub fn get(&self, name: &str) -> Option<&dyn OAuthProvider> {
        self.providers.get(name).map(|p| p.as_ref())
    }

    /// Get the list of registered provider names
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

// ── PKCE utilities ───────────────────────────────────────────

/// Generate a random code_verifier (43 characters, satisfying the 43-128 requirement)
pub fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .unwrap_or_else(|e| panic!("code_verifier generation failed: {e}"));
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Generate a code_challenge from a code_verifier (S256 method)
pub fn generate_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Generate a random state parameter (32-byte hex)
pub fn generate_state() -> String {
    crate::utils::id::random_hex(32)
}

// ── Tests ────────────────────────────────────────────────────

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

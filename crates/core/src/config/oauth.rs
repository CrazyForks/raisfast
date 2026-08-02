//! OAuth2 social login configuration
//!
//! Supports multiple providers (GitHub, Google, WeChat), configured via environment variables.

use serde::{Deserialize, Serialize};

/// GitHub OAuth configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// Google OAuth configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// WeChat OAuth configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatOAuthConfig {
    pub app_id: String,
    pub app_secret: String,
}

/// OAuth2 overall configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConfig {
    /// Whether to enable OAuth (default false)
    pub enabled: bool,
    /// Frontend callback URL (302 redirect target after successful login)
    pub redirect_url: String,
    /// GitHub configuration
    pub github: Option<GitHubOAuthConfig>,
    /// Google configuration
    pub google: Option<GoogleOAuthConfig>,
    /// WeChat configuration
    pub wechat: Option<WechatOAuthConfig>,
}

impl OAuthConfig {
    /// Load from environment variables, using defaults for missing items
    pub fn from_env() -> Self {
        let enabled = std::env::var("OAUTH_ENABLED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(false);

        let redirect_url = std::env::var("OAUTH_REDIRECT_URL")
            .unwrap_or_else(|_| "http://localhost:3000/auth/callback".into());

        let github = {
            let client_id = std::env::var("OAUTH_GITHUB_CLIENT_ID").ok();
            let client_secret = std::env::var("OAUTH_GITHUB_CLIENT_SECRET").ok();
            match (client_id, client_secret) {
                (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
                    Some(GitHubOAuthConfig {
                        client_id: id,
                        client_secret: secret,
                    })
                }
                _ => None,
            }
        };

        let google = {
            let client_id = std::env::var("OAUTH_GOOGLE_CLIENT_ID").ok();
            let client_secret = std::env::var("OAUTH_GOOGLE_CLIENT_SECRET").ok();
            match (client_id, client_secret) {
                (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
                    Some(GoogleOAuthConfig {
                        client_id: id,
                        client_secret: secret,
                    })
                }
                _ => None,
            }
        };

        let wechat = {
            let app_id = std::env::var("OAUTH_WECHAT_APP_ID").ok();
            let app_secret = std::env::var("OAUTH_WECHAT_APP_SECRET").ok();
            match (app_id, app_secret) {
                (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
                    Some(WechatOAuthConfig {
                        app_id: id,
                        app_secret: secret,
                    })
                }
                _ => None,
            }
        };

        Self {
            enabled,
            redirect_url,
            github,
            google,
            wechat,
        }
    }

    /// Check if the specified provider is configured
    pub fn is_provider_configured(&self, provider: &str) -> bool {
        match provider {
            "github" => self.github.is_some(),
            "google" => self.google.is_some(),
            "wechat" => self.wechat.is_some(),
            _ => false,
        }
    }

    /// Get the list of configured provider names
    pub fn configured_providers(&self) -> Vec<&str> {
        let mut providers = Vec::new();
        if self.github.is_some() {
            providers.push("github");
        }
        if self.google.is_some() {
            providers.push("google");
        }
        if self.wechat.is_some() {
            providers.push("wechat");
        }
        providers
    }
}

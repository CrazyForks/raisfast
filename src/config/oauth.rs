//! OAuth2 社交登录配置
//!
//! 支持多个 Provider（GitHub、Google、微信），通过环境变量配置。

use serde::{Deserialize, Serialize};

/// GitHub OAuth 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// Google OAuth 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// 微信 OAuth 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatOAuthConfig {
    pub app_id: String,
    pub app_secret: String,
}

/// OAuth2 总配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConfig {
    /// 是否启用 OAuth 功能（默认 false）
    pub enabled: bool,
    /// 前端回调地址（登录成功后 302 重定向目标）
    pub redirect_url: String,
    /// GitHub 配置
    pub github: Option<GitHubOAuthConfig>,
    /// Google 配置
    pub google: Option<GoogleOAuthConfig>,
    /// 微信配置
    pub wechat: Option<WechatOAuthConfig>,
}

impl OAuthConfig {
    /// 从环境变量加载，缺失项使用默认值
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

    /// 检查指定 Provider 是否已配置
    pub fn is_provider_configured(&self, provider: &str) -> bool {
        match provider {
            "github" => self.github.is_some(),
            "google" => self.google.is_some(),
            "wechat" => self.wechat.is_some(),
            _ => false,
        }
    }

    /// 获取已配置的 Provider 名称列表
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

//! GitHub OAuth2 Provider implementation

use crate::errors::app_error::{AppError, AppResult};
use crate::oauth::{OAuthProvider, OAuthTokenResponse, OAuthUserInfo};

/// GitHub OAuth2 Provider
pub struct GitHubProvider {
    client_id: String,
    client_secret: String,
}

impl GitHubProvider {
    /// Create a GitHub Provider
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
        }
    }

    /// Fetch the GitHub user's primary email (requires user:email scope)
    async fn fetch_primary_email(
        &self,
        client: &reqwest::Client,
        access_token: &str,
    ) -> Option<String> {
        let resp = client
            .get("https://api.github.com/user/emails")
            .header(
                crate::constants::HEADER_AUTHORIZATION,
                format!("{}{access_token}", crate::constants::AUTH_BEARER_PREFIX),
            )
            .header("User-Agent", "raisfast")
            .header("Accept", "application/json")
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let emails: Vec<serde_json::Value> = resp.json().await.ok()?;
        emails
            .iter()
            .find(|e| e["primary"].as_bool() == Some(true) && e["verified"].as_bool() == Some(true))
            .and_then(|e| e["email"].as_str().map(|s| s.to_string()))
    }
}

#[async_trait::async_trait]
impl OAuthProvider for GitHubProvider {
    fn name(&self) -> &str {
        "github"
    }

    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        format!(
            "https://github.com/login/oauth/authorize?client_id={}&state={}&code_challenge={}&code_challenge_method=S256&scope=user:email",
            self.client_id, state, code_challenge
        )
    }

    async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> AppResult<OAuthTokenResponse> {
        let client = super::http_client();
        let resp = client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "code": code,
                "code_verifier": code_verifier,
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::Error::from(e).context("GitHub token exchange failed"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "GitHub token exchange returned {status}: {body}"
            )));
        }

        resp.json::<OAuthTokenResponse>().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("GitHub token response parse failed"))
        })
    }

    async fn fetch_user_info(&self, access_token: &str) -> AppResult<OAuthUserInfo> {
        let client = super::http_client();

        let resp = client
            .get("https://api.github.com/user")
            .header(
                crate::constants::HEADER_AUTHORIZATION,
                format!("{}{access_token}", crate::constants::AUTH_BEARER_PREFIX),
            )
            .header("User-Agent", "raisfast")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(
                    anyhow::Error::from(e).context("GitHub user info request failed"),
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "GitHub user info returned {status}: {body}"
            )));
        }

        let profile: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("GitHub user info parse failed"))
        })?;

        let provider_user_id = profile["id"]
            .as_i64()
            .map(|i| i.to_string())
            .unwrap_or_default();
        let display_name = profile["login"].as_str().map(|s| s.to_string());
        let avatar_url = profile["avatar_url"].as_str().map(|s| s.to_string());

        let email = if let Some(email) = profile["email"].as_str() {
            if !email.is_empty() {
                Some(email.to_string())
            } else {
                self.fetch_primary_email(&client, access_token).await
            }
        } else {
            self.fetch_primary_email(&client, access_token).await
        };

        Ok(OAuthUserInfo {
            provider_user_id,
            email,
            display_name,
            avatar_url,
            raw_profile: profile,
        })
    }
}

//! Google OAuth2 Provider implementation

use crate::errors::app_error::{AppError, AppResult};
use crate::oauth::{OAuthProvider, OAuthTokenResponse, OAuthUserInfo};

/// Google OAuth2 Provider
pub struct GoogleProvider {
    client_id: String,
    client_secret: String,
}

impl GoogleProvider {
    /// Create a Google Provider
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
        }
    }
}

#[async_trait::async_trait]
impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
    }

    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&state={}&code_challenge={}&code_challenge_method=S256&scope=openid+email+profile&response_type=code&access_type=offline",
            self.client_id, state, code_challenge
        )
    }

    async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> AppResult<OAuthTokenResponse> {
        let resp = super::http_client()
            .post("https://oauth2.googleapis.com/token")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "code": code,
                "code_verifier": code_verifier,
                "grant_type": "authorization_code",
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::Error::from(e).context("Google token exchange failed"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "Google token exchange returned {status}: {body}"
            )));
        }

        resp.json::<OAuthTokenResponse>().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("Google token response parse failed"))
        })
    }

    async fn fetch_user_info(&self, access_token: &str) -> AppResult<OAuthUserInfo> {
        let resp = super::http_client()
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .header(
                crate::constants::HEADER_AUTHORIZATION,
                format!("{}{access_token}", crate::constants::AUTH_BEARER_PREFIX),
            )
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(
                    anyhow::Error::from(e).context("Google user info request failed"),
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "Google user info returned {status}: {body}"
            )));
        }

        let profile: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("Google user info parse failed"))
        })?;

        let provider_user_id = profile["sub"].as_str().unwrap_or_default().to_string();
        let email = profile["email"].as_str().map(|s| s.to_string());
        let display_name = profile["name"]
            .as_str()
            .or_else(|| profile["given_name"].as_str())
            .map(|s| s.to_string());
        let avatar_url = profile["picture"].as_str().map(|s| s.to_string());

        Ok(OAuthUserInfo {
            provider_user_id,
            email,
            display_name,
            avatar_url,
            raw_profile: profile,
        })
    }
}

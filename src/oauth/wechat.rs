//! WeChat Open Platform OAuth2 Provider implementation
//!
//! WeChat website application QR code login flow:
//!
//! 1. Frontend displays a WeChat QR code (`CONNECT_URL` + `appid` + `redirect_uri`)
//! 2. After the user scans and authorizes, WeChat redirects back with a `code`
//! 3. Backend exchanges `code` for `access_token` + `openid`
//! 4. Uses `access_token` + `openid` to fetch user info (nickname, avatar)
//!
//! WeChat OAuth does not support PKCE; the `code_challenge` parameter is ignored.

use crate::errors::app_error::{AppError, AppResult};
use crate::oauth::{OAuthProvider, OAuthTokenResponse, OAuthUserInfo};

/// WeChat Open Platform OAuth2 Provider
pub struct WechatProvider {
    app_id: String,
    app_secret: String,
    base_url: String,
}

impl WechatProvider {
    /// Create a WeChat Provider
    pub fn new(app_id: String, app_secret: String, base_url: String) -> Self {
        Self {
            app_id,
            app_secret,
            base_url,
        }
    }
}

#[async_trait::async_trait]
impl OAuthProvider for WechatProvider {
    fn name(&self) -> &str {
        "wechat"
    }

    /// Build the WeChat QR code login authorize URL
    ///
    /// Note: WeChat does not support PKCE; `code_challenge` is ignored.
    /// The frontend can also use this URL directly to display the QR code.
    fn authorize_url(&self, state: &str, _code_challenge: &str) -> String {
        let callback = format!("{}/api/v1/auth/oauth/callback/wechat", self.base_url);
        let redirect_uri = urlencoding::encode(&callback);
        format!(
            "https://open.weixin.qq.com/connect/qrconnect?appid={}&redirect_uri={}&response_type=code&scope=snsapi_login&state={}#wechat_redirect",
            self.app_id, redirect_uri, state
        )
    }

    /// Exchange code for access_token + openid
    ///
    /// The WeChat token endpoint returns JSON (not standard OAuth2 format) and requires adaptation.
    async fn exchange_code(
        &self,
        code: &str,
        _code_verifier: &str,
    ) -> AppResult<OAuthTokenResponse> {
        let client = super::http_client();

        let url = format!(
            "https://api.weixin.qq.com/sns/oauth2/access_token?appid={}&secret={}&code={}&grant_type=authorization_code",
            self.app_id, self.app_secret, code
        );

        let resp = client.get(&url).send().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("WeChat token exchange failed"))
        })?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("WeChat token response parse failed"))
        })?;

        if let Some(errcode) = body["errcode"].as_i64() {
            let errmsg = body["errmsg"].as_str().unwrap_or("unknown");
            return Err(AppError::Internal(anyhow::anyhow!(
                "WeChat API error: errcode={errcode}, errmsg={errmsg}"
            )));
        }

        let access_token = body["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        let openid = body["openid"].as_str().unwrap_or_default().to_string();

        Ok(OAuthTokenResponse {
            access_token: format!("{access_token}:{openid}"),
            token_type: Some("Bearer".into()),
            refresh_token: body["refresh_token"].as_str().map(|s| s.to_string()),
            expires_in: body["expires_in"].as_u64(),
            scope: body["scope"].as_str().map(|s| s.to_string()),
        })
    }

    /// Fetch user info using access_token + openid
    ///
    /// The access_token format is `{token}:{openid}` (concatenated in exchange_code).
    async fn fetch_user_info(&self, combined_token: &str) -> AppResult<OAuthUserInfo> {
        let (access_token, openid) = combined_token
            .rsplit_once(':')
            .unwrap_or((combined_token, ""));

        let url = format!(
            "https://api.weixin.qq.com/sns/userinfo?access_token={access_token}&openid={openid}"
        );

        let client = super::http_client();
        let resp = client.get(&url).send().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("WeChat user info request failed"))
        })?;

        let profile: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("WeChat user info parse failed"))
        })?;

        if let Some(errcode) = profile["errcode"].as_i64() {
            let errmsg = profile["errmsg"].as_str().unwrap_or("unknown");
            return Err(AppError::Internal(anyhow::anyhow!(
                "WeChat userinfo API error: errcode={errcode}, errmsg={errmsg}"
            )));
        }

        let provider_user_id = profile["openid"].as_str().unwrap_or_default().to_string();
        let display_name = profile["nickname"].as_str().map(|s| s.to_string());
        let avatar_url = profile["headimgurl"].as_str().map(|s| s.to_string());

        let unionid = profile["unionid"].as_str().map(|s| s.to_string());

        Ok(OAuthUserInfo {
            provider_user_id: unionid.unwrap_or(provider_user_id),
            email: None,
            display_name,
            avatar_url,
            raw_profile: profile,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wechat_authorize_url_format() {
        let provider = WechatProvider::new(
            "wx12345".into(),
            "secret".into(),
            "http://localhost:9000".into(),
        );
        let url = provider.authorize_url("state123", "challenge456");
        assert!(url.contains("appid=wx12345"));
        assert!(url.contains("state=state123"));
        assert!(url.contains("scope=snsapi_login"));
        assert!(url.contains("connect/qrconnect"));
        assert!(!url.contains("code_challenge"));
    }

    #[test]
    fn wechat_name() {
        let provider = WechatProvider::new(
            "wx12345".into(),
            "secret".into(),
            "http://localhost:9000".into(),
        );
        assert_eq!(provider.name(), "wechat");
    }
}

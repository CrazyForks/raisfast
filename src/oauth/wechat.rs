//! 微信开放平台 OAuth2 Provider 实现
//!
//! 微信网站应用扫码登录流程：
//!
//! 1. 前端展示微信二维码（`CONNECT_URL` + `appid` + `redirect_uri`）
//! 2. 用户扫码授权后，微信回调带 `code`
//! 3. 后端用 `code` 换 `access_token` + `openid`
//! 4. 用 `access_token` + `openid` 获取用户信息（昵称、头像）
//!
//! 微信 OAuth 不支持 PKCE，`code_challenge` 参数被忽略。

use crate::errors::app_error::{AppError, AppResult};
use crate::oauth::{OAuthProvider, OAuthTokenResponse, OAuthUserInfo};

/// 微信开放平台 OAuth2 Provider
pub struct WechatProvider {
    app_id: String,
    app_secret: String,
    base_url: String,
}

impl WechatProvider {
    /// 创建微信 Provider
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

    /// 构建微信扫码登录授权 URL
    ///
    /// 注意：微信不支持 PKCE，`code_challenge` 被忽略。
    /// 前端也可以直接用此 URL 展示二维码。
    fn authorize_url(&self, state: &str, _code_challenge: &str) -> String {
        let callback = format!("{}/api/v1/auth/oauth/callback/wechat", self.base_url);
        let redirect_uri = urlencoding::encode(&callback);
        format!(
            "https://open.weixin.qq.com/connect/qrconnect?appid={}&redirect_uri={}&response_type=code&scope=snsapi_login&state={}#wechat_redirect",
            self.app_id, redirect_uri, state
        )
    }

    /// 用 code 换 access_token + openid
    ///
    /// 微信 token 接口返回 JSON（不是标准 OAuth2 格式），需要适配。
    async fn exchange_code(
        &self,
        code: &str,
        _code_verifier: &str,
    ) -> AppResult<OAuthTokenResponse> {
        let client = reqwest::Client::new();

        let url = format!(
            "https://api.weixin.qq.com/sns/oauth2/access_token?appid={}&secret={}&code={}&grant_type=authorization_code",
            self.app_id, self.app_secret, code
        );

        let resp = client.get(&url).send().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("WeChat token exchange failed: {e}"))
        })?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("WeChat token response parse failed: {e}"))
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

    /// 用 access_token + openid 获取用户信息
    ///
    /// access_token 格式为 `{token}:{openid}`（在 exchange_code 中拼接）。
    async fn fetch_user_info(&self, combined_token: &str) -> AppResult<OAuthUserInfo> {
        let (access_token, openid) = combined_token
            .rsplit_once(':')
            .unwrap_or((combined_token, ""));

        let url = format!(
            "https://api.weixin.qq.com/sns/userinfo?access_token={access_token}&openid={openid}"
        );

        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("WeChat user info request failed: {e}"))
        })?;

        let profile: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("WeChat user info parse failed: {e}"))
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

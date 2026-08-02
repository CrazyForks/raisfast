use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use x509_cert::der::DecodePem as _;

use crate::errors::app_error::{AppError, AppResult};
use crate::models::payment_channel::PaymentChannel;
use crate::models::payment_order::{PaymentOrder, PaymentStatus};
use crate::payment::PaymentProvider;
use crate::payment::crypto::aes256gcm_decrypt;
use crate::payment::provider::{CallbackData, ProviderResponse, ProviderStatus, RefundResponse};

const BASE_URL: &str = "https://api.mch.weixin.qq.com";

#[derive(Deserialize)]
struct WechatCredentials {
    appid: String,
    mchid: String,
    private_key: String,
    serial_no: String,
}

#[derive(Serialize)]
struct NativeRequest {
    appid: String,
    mchid: String,
    description: String,
    out_trade_no: String,
    time_expire: Option<String>,
    notify_url: Option<String>,
    amount: NativeAmount,
}

#[derive(Serialize)]
struct NativeAmount {
    total: i64,
    currency: String,
}

#[derive(Deserialize)]
struct NativeResponse {
    code_url: Option<String>,
}

#[derive(Deserialize)]
struct QueryResponse {
    trade_state: Option<String>,
    transaction_id: Option<String>,
    success_time: Option<String>,
    amount: Option<QueryAmount>,
}

#[derive(Deserialize)]
struct QueryAmount {
    total: Option<i64>,
}

#[derive(Serialize)]
struct RefundBody {
    out_trade_no: String,
    out_refund_no: String,
    amount: RefundAmountBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Serialize)]
struct RefundAmountBody {
    refund: i64,
    total: i64,
    currency: String,
}

#[derive(Deserialize)]
struct RefundResponseData {
    refund_id: Option<String>,
    out_refund_no: Option<String>,
}

#[derive(Deserialize)]
struct WechatCallbackResource {
    ciphertext: String,
    nonce: String,
    associated_data: String,
}

#[derive(Deserialize)]
struct WechatCallbackEvent {
    resource: Option<WechatCallbackResource>,
}

#[derive(Deserialize)]
struct DecryptedResource {
    out_trade_no: Option<String>,
    trade_state: Option<String>,
    transaction_id: Option<String>,
    success_time: Option<String>,
    amount: Option<ResourceAmount>,
}

#[derive(Deserialize)]
struct ResourceAmount {
    total: Option<i64>,
}

fn decrypt_credentials(
    channel: &PaymentChannel,
    encrypt_key: &[u8; 32],
) -> AppResult<WechatCredentials> {
    let decrypted = aes256gcm_decrypt(&channel.credentials, encrypt_key)?;
    serde_json::from_str(&decrypted)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat credentials parse")))
}

fn parse_private_key(pem_or_raw: &str) -> AppResult<rsa::RsaPrivateKey> {
    let pem = if pem_or_raw.starts_with("-----BEGIN") {
        pem_or_raw.to_string()
    } else {
        format!(
            "-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----",
            pem_or_raw
        )
    };
    rsa::RsaPrivateKey::from_pkcs1_pem(&pem)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat private key parse")))
}

fn extract_platform_cert(channel: &PaymentChannel) -> AppResult<String> {
    channel
        .webhook_secret
        .as_deref()
        .ok_or_else(|| {
            AppError::BadRequest(
                "wechat: platform certificate (webhook_secret) not configured".into(),
            )
        })
        .map(String::from)
}

fn parse_platform_cert_pub_key(pem: &str) -> AppResult<rsa::RsaPublicKey> {
    let pem_str = if pem.starts_with("-----BEGIN") {
        pem.to_string()
    } else {
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----",
            pem
        )
    };
    let cert = x509_cert::Certificate::from_pem(pem_str)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat cert parse")))?;
    let pub_key_bytes = cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("wechat cert: invalid public key bitstring"))
        })?;
    rsa::RsaPublicKey::from_public_key_der(pub_key_bytes)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat cert public key")))
}

fn rsa_sign(message: &str, private_key: &rsa::RsaPrivateKey) -> AppResult<String> {
    let hash = Sha256::digest(message.as_bytes());
    let scheme = rsa::pkcs1v15::Pkcs1v15Sign::new::<Sha256>();
    let sig = private_key
        .sign(scheme, &hash)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat sign")))?;
    Ok(BASE64.encode(sig))
}

fn rsa_verify(message: &str, signature_b64: &str, public_key: &rsa::RsaPublicKey) -> AppResult<()> {
    let sig_bytes = BASE64.decode(signature_b64).map_err(|e| {
        AppError::Internal(anyhow::Error::from(e).context("wechat signature base64 decode"))
    })?;
    let hash = Sha256::digest(message.as_bytes());
    let scheme = rsa::pkcs1v15::Pkcs1v15Sign::new::<Sha256>();
    public_key.verify(scheme, &hash, &sig_bytes).map_err(|e| {
        tracing::warn!("wechat signature verification failed: {e}");
        AppError::BadRequest("wechat signature mismatch".into())
    })
}

fn build_auth_header(mchid: &str, serial_no: &str, signature: &str) -> String {
    format!(
        r#"WECHATPAY2-SHA256-RSA2048 mchid="{}",nonce_str="{}",timestamp="{}",serial_no="{}",signature="{}""#,
        mchid,
        generate_nonce(),
        chrono::Utc::now().timestamp(),
        serial_no,
        signature
    )
}

fn generate_nonce() -> String {
    use std::fmt::Write;
    let mut bytes = [0u8; 16];
    let _ = getrandom::getrandom(&mut bytes);
    let mut s = String::with_capacity(32);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn trade_state_to_payment(state: &str) -> PaymentStatus {
    match state {
        "SUCCESS" => PaymentStatus::Paid,
        "CLOSED" => PaymentStatus::Cancelled,
        "REFUND" => PaymentStatus::Refunded,
        "NOTPAY" | "USERPAYING" => PaymentStatus::Pending,
        "PAYERROR" => PaymentStatus::Failed,
        _ => PaymentStatus::Pending,
    }
}

async fn wechat_request<T: serde::de::DeserializeOwned>(
    creds: &WechatCredentials,
    private_key: &rsa::RsaPrivateKey,
    method: &str,
    path: &str,
    body: Option<String>,
) -> AppResult<T> {
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let nonce = generate_nonce();
    let body_str = body.as_deref().unwrap_or("");
    let signing_str = format!("{method}\n{path}\n{timestamp}\n{nonce}\n{body_str}\n");
    let signature = rsa_sign(&signing_str, private_key)?;
    let auth = build_auth_header(&creds.mchid, &creds.serial_no, &signature);

    let url = format!("{BASE_URL}{path}");
    let client = super::http_client();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "unsupported wechat http method: {method}"
            )));
        }
    };
    req = req
        .header("Authorization", &auth)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json");

    if let Some(b) = body {
        req = req.body(b);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat request")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat read body")))?;

    if !status.is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "wechat api error: {status} - {text}"
        )));
    }

    serde_json::from_str::<T>(&text)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat response parse")))
}

fn decrypt_callback_resource(
    ciphertext_b64: &str,
    nonce: &str,
    associated_data: &str,
    api_key: &str,
) -> AppResult<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let key = Sha256::digest(api_key.as_bytes());
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("wechat aes init: {e:?}")))?;
    let ciphertext = BASE64.decode(ciphertext_b64).map_err(|e| {
        AppError::Internal(anyhow::Error::from(e).context("wechat resource base64 decode"))
    })?;
    let nonce = Nonce::from_slice(nonce.as_bytes());
    let plaintext = cipher
        .decrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: ciphertext.as_slice(),
                aad: associated_data.as_bytes(),
            },
        )
        .map_err(|e| AppError::Internal(anyhow::anyhow!("wechat resource decrypt: {e:?}")))?;
    String::from_utf8(plaintext)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("wechat resource utf8")))
}

fn extract_notify_url(channel: &PaymentChannel) -> Option<String> {
    channel
        .settings
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("notify_url").cloned())
        .and_then(|v| v.as_str().map(String::from))
}

pub struct WechatPayProvider {
    encrypt_key: [u8; 32],
}

impl WechatPayProvider {
    pub fn new(encrypt_key: [u8; 32]) -> Self {
        Self { encrypt_key }
    }
}

#[async_trait::async_trait]
impl PaymentProvider for WechatPayProvider {
    fn name(&self) -> &str {
        "wechat"
    }

    async fn create(
        &self,
        channel: &PaymentChannel,
        order: &PaymentOrder,
        _return_url: Option<&str>,
        _notify_url: Option<&str>,
    ) -> AppResult<ProviderResponse> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let private_key = parse_private_key(&creds.private_key)?;

        let expire = chrono::Utc::now() + chrono::Duration::minutes(30);
        let req = NativeRequest {
            appid: creds.appid.clone(),
            mchid: creds.mchid.clone(),
            description: order.title.clone(),
            out_trade_no: order.id.to_string(),
            time_expire: Some(expire.to_rfc3339()),
            notify_url: extract_notify_url(channel),
            amount: NativeAmount {
                total: order.amount,
                currency: order.currency.to_uppercase(),
            },
        };
        let body_str = serde_json::to_string(&req).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("wechat request serialize"))
        })?;

        let resp: NativeResponse = wechat_request(
            &creds,
            &private_key,
            "POST",
            "/v3/pay/transactions/native",
            Some(body_str),
        )
        .await?;

        Ok(ProviderResponse {
            provider_order_id: order.id.to_string(),
            redirect_url: None,
            qr_code: resp.code_url,
            client_secret: None,
        })
    }

    async fn query(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
    ) -> AppResult<ProviderStatus> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let private_key = parse_private_key(&creds.private_key)?;

        let path = format!(
            "/v3/pay/transactions/out-trade-no/{}?mchid={}",
            provider_order_id, creds.mchid
        );

        let resp: QueryResponse = wechat_request(&creds, &private_key, "GET", &path, None).await?;

        let status = resp
            .trade_state
            .as_deref()
            .map(trade_state_to_payment)
            .unwrap_or(PaymentStatus::Pending);

        Ok(ProviderStatus {
            status,
            provider_tx_id: resp.transaction_id,
            paid_at: resp.success_time,
            amount: resp.amount.and_then(|a| a.total),
        })
    }

    async fn cancel(&self, channel: &PaymentChannel, provider_order_id: &str) -> AppResult<()> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let private_key = parse_private_key(&creds.private_key)?;

        let path = format!(
            "/v3/pay/transactions/out-trade-no/{}/close",
            provider_order_id
        );
        let body_str = serde_json::json!({
            "mchid": creds.mchid
        })
        .to_string();

        let _: serde_json::Value =
            wechat_request(&creds, &private_key, "POST", &path, Some(body_str)).await?;

        Ok(())
    }

    async fn refund(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
        amount: i64,
        reason: Option<&str>,
    ) -> AppResult<RefundResponse> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let private_key = parse_private_key(&creds.private_key)?;

        let refund_no = format!("refund_{}", uuid::Uuid::now_v7());

        let query_path = format!(
            "/v3/pay/transactions/out-trade-no/{}?mchid={}",
            provider_order_id, creds.mchid
        );
        let query_resp: QueryResponse =
            wechat_request(&creds, &private_key, "GET", &query_path, None).await?;

        let total_amount = query_resp
            .amount
            .as_ref()
            .and_then(|a| a.total)
            .unwrap_or(amount);

        let body = RefundBody {
            out_trade_no: provider_order_id.to_string(),
            out_refund_no: refund_no.clone(),
            amount: RefundAmountBody {
                refund: amount,
                total: total_amount,
                currency: "CNY".to_string(),
            },
            reason: reason.map(String::from),
        };
        let body_str = serde_json::to_string(&body).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("wechat refund serialize"))
        })?;

        let resp: RefundResponseData = wechat_request(
            &creds,
            &private_key,
            "POST",
            "/v3/refund/domestic/refunds",
            Some(body_str),
        )
        .await?;

        Ok(RefundResponse {
            provider_refund_id: resp.refund_id.or(resp.out_refund_no).unwrap_or(refund_no),
        })
    }

    async fn verify_callback(
        &self,
        channel: &PaymentChannel,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<CallbackData> {
        let wechat_timestamp = headers
            .get("Wechatpay-Timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("wechat: missing Wechatpay-Timestamp".into()))?;
        let wechat_nonce = headers
            .get("Wechatpay-Nonce")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("wechat: missing Wechatpay-Nonce".into()))?;
        let wechat_signature = headers
            .get("Wechatpay-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("wechat: missing Wechatpay-Signature".into()))?;

        let body_str = std::str::from_utf8(body)
            .map_err(|e| AppError::BadRequest(format!("wechat callback: invalid utf8: {e}")))?;

        let verify_msg = format!("{wechat_timestamp}\n{wechat_nonce}\n{body_str}\n");

        let cert_pem = extract_platform_cert(channel)?;
        let pub_key = parse_platform_cert_pub_key(&cert_pem)?;

        rsa_verify(&verify_msg, wechat_signature, &pub_key)?;

        let event: WechatCallbackEvent = serde_json::from_str(body_str)
            .map_err(|e| AppError::BadRequest(format!("wechat callback parse: {e}")))?;

        let resource = event
            .resource
            .ok_or_else(|| AppError::BadRequest("wechat callback: missing resource".into()))?;

        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let decrypted = decrypt_callback_resource(
            &resource.ciphertext,
            &resource.nonce,
            &resource.associated_data,
            &creds.mchid,
        )?;

        let resource_data: DecryptedResource = serde_json::from_str(&decrypted)
            .map_err(|e| AppError::BadRequest(format!("wechat resource parse: {e}")))?;

        let status = resource_data
            .trade_state
            .as_deref()
            .map(trade_state_to_payment)
            .unwrap_or(PaymentStatus::Pending);

        let amount = resource_data.amount.and_then(|a| a.total).unwrap_or(0);

        Ok(CallbackData {
            provider_order_id: resource_data.out_trade_no.unwrap_or_default(),
            status,
            amount,
            provider_tx_id: resource_data.transaction_id,
            paid_at: resource_data.success_time,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_state_mapping() {
        assert!(matches!(
            trade_state_to_payment("SUCCESS"),
            PaymentStatus::Paid
        ));
        assert!(matches!(
            trade_state_to_payment("CLOSED"),
            PaymentStatus::Cancelled
        ));
        assert!(matches!(
            trade_state_to_payment("REFUND"),
            PaymentStatus::Refunded
        ));
        assert!(matches!(
            trade_state_to_payment("NOTPAY"),
            PaymentStatus::Pending
        ));
        assert!(matches!(
            trade_state_to_payment("USERPAYING"),
            PaymentStatus::Pending
        ));
        assert!(matches!(
            trade_state_to_payment("PAYERROR"),
            PaymentStatus::Failed
        ));
        assert!(matches!(
            trade_state_to_payment("UNKNOWN"),
            PaymentStatus::Pending
        ));
    }

    #[test]
    fn generate_nonce_length() {
        let nonce = generate_nonce();
        assert_eq!(nonce.len(), 32);
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn build_auth_header_format() {
        let auth = build_auth_header("mch123", "serial456", "sig789");
        assert!(auth.starts_with("WECHATPAY2-SHA256-RSA2048"));
        assert!(auth.contains(r#"mchid="mch123""#));
        assert!(auth.contains(r#"serial_no="serial456""#));
        assert!(auth.contains(r#"signature="sig789""#));
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let mut rng = rsa::rand_core::OsRng;
        let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = rsa::RsaPublicKey::from(&private_key);

        let message = "POST\n/v3/pay/transactions/native\n1234567890\nabc123\n{}\n";
        let signature = rsa_sign(message, &private_key).unwrap();
        rsa_verify(message, &signature, &public_key).unwrap();
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let mut rng = rsa::rand_core::OsRng;
        let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = rsa::RsaPublicKey::from(&private_key);

        let message = "original message";
        let signature = rsa_sign(message, &private_key).unwrap();
        assert!(rsa_verify("tampered message", &signature, &public_key).is_err());
    }

    #[test]
    fn extract_notify_url_works() {
        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "wechat".into(),
            name: "Wechat".into(),
            is_live: 0,
            credentials: String::new(),
            webhook_secret: None,
            settings: Some(r#"{"notify_url":"https://example.com/callback"}"#.into()),
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: crate::utils::tz::Timestamp::default(),
            updated_at: crate::utils::tz::Timestamp::default(),
        };
        assert_eq!(
            extract_notify_url(&channel),
            Some("https://example.com/callback".to_string())
        );
    }

    #[test]
    fn extract_notify_url_missing() {
        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "wechat".into(),
            name: "Wechat".into(),
            is_live: 0,
            credentials: String::new(),
            webhook_secret: None,
            settings: None,
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: crate::utils::tz::Timestamp::default(),
            updated_at: crate::utils::tz::Timestamp::default(),
        };
        assert_eq!(extract_notify_url(&channel), None);
    }
}

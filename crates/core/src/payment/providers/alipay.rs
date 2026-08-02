use std::collections::BTreeMap;

use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::pkcs8::DecodePrivateKey as DecodePkcs8PrivateKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::models::payment_channel::PaymentChannel;
use crate::models::payment_order::{PaymentOrder, PaymentStatus};
use crate::payment::PaymentProvider;
use crate::payment::crypto::aes256gcm_decrypt;
use crate::payment::provider::{CallbackData, ProviderResponse, ProviderStatus, RefundResponse};

const GATEWAY_LIVE: &str = "https://openapi.alipay.com/gateway.do";
const GATEWAY_TEST: &str = "https://openapi-sandbox.dl.alipaydev.com/gateway.do";

const CHARSET: &str = "utf-8";
const SIGN_TYPE: &str = "RSA2";
const VERSION: &str = "1.0";

#[derive(Deserialize)]
struct AlipayCredentials {
    app_id: String,
    private_key: String,
    #[serde(default)]
    test_mode: bool,
}

#[derive(Serialize)]
struct PrecreateBizContent {
    out_trade_no: String,
    total_amount: String,
    subject: String,
    timeout_express: Option<String>,
}

#[derive(Serialize)]
struct QueryBizContent {
    out_trade_no: String,
}

#[derive(Serialize)]
struct CancelBizContent {
    out_trade_no: String,
}

#[derive(Serialize)]
struct RefundBizContent {
    out_trade_no: String,
    refund_amount: String,
    out_request_no: Option<String>,
    refund_reason: Option<String>,
}

#[derive(Deserialize)]
struct AlipayResponse<T> {
    #[serde(rename = "alipay_trade_precreate_response")]
    precreate: Option<T>,
    #[serde(rename = "alipay_trade_query_response")]
    query: Option<T>,
    #[serde(rename = "alipay_trade_cancel_response")]
    cancel: Option<T>,
    #[serde(rename = "alipay_trade_refund_response")]
    refund: Option<T>,
}

#[derive(Deserialize)]
struct PrecreateResult {
    out_trade_no: Option<String>,
    qr_code: Option<String>,
    trade_no: Option<String>,
    code: Option<String>,
    sub_code: Option<String>,
    sub_msg: Option<String>,
}

#[derive(Deserialize)]
struct QueryResult {
    trade_status: Option<String>,
    trade_no: Option<String>,
    send_pay_date: Option<String>,
    code: Option<String>,
}

#[derive(Deserialize)]
struct CancelResult {
    code: Option<String>,
}

#[derive(Deserialize)]
struct RefundResult {
    code: Option<String>,
    trade_no: Option<String>,
    sub_code: Option<String>,
    sub_msg: Option<String>,
}

fn decrypt_credentials(
    channel: &PaymentChannel,
    encrypt_key: &[u8; 32],
) -> AppResult<AlipayCredentials> {
    let decrypted = aes256gcm_decrypt(&channel.credentials, encrypt_key)?;
    serde_json::from_str(&decrypted)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("alipay credentials parse")))
}

fn wrap_pem(header: &str, footer: &str, raw: &str) -> String {
    let clean: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let mut lines = Vec::new();
    for chunk in clean.as_bytes().chunks(64) {
        lines.push(String::from_utf8_lossy(chunk).to_string());
    }
    format!("{}\n{}\n{}", header, lines.join("\n"), footer)
}

fn parse_private_key(pem_or_raw: &str) -> AppResult<rsa::RsaPrivateKey> {
    let trimmed = pem_or_raw.trim();
    let pem = if trimmed.starts_with("-----BEGIN") {
        trimmed.to_string()
    } else {
        wrap_pem(
            "-----BEGIN PRIVATE KEY-----",
            "-----END PRIVATE KEY-----",
            trimmed,
        )
    };
    rsa::RsaPrivateKey::from_pkcs8_pem(&pem)
        .or_else(|_| {
            let pem = if trimmed.starts_with("-----BEGIN") {
                trimmed.to_string()
            } else {
                wrap_pem(
                    "-----BEGIN RSA PRIVATE KEY-----",
                    "-----END RSA PRIVATE KEY-----",
                    trimmed,
                )
            };
            rsa::RsaPrivateKey::from_pkcs1_pem(&pem)
        })
        .map_err(|e| AppError::Internal(anyhow::anyhow!("alipay private key parse: {e}")))
}

fn parse_public_key(pem_or_raw: &str) -> AppResult<rsa::RsaPublicKey> {
    let trimmed = pem_or_raw.trim();
    let pem = if trimmed.starts_with("-----BEGIN") {
        trimmed.to_string()
    } else {
        wrap_pem(
            "-----BEGIN PUBLIC KEY-----",
            "-----END PUBLIC KEY-----",
            trimmed,
        )
    };
    rsa::RsaPublicKey::from_public_key_pem(&pem)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("alipay public key parse: {e}")))
}

fn gateway_url(creds: &AlipayCredentials) -> &str {
    if creds.test_mode {
        GATEWAY_TEST
    } else {
        GATEWAY_LIVE
    }
}

fn sign_data(data: &[u8], private_key: &rsa::RsaPrivateKey) -> AppResult<Vec<u8>> {
    let hash = Sha256::digest(data);
    let scheme = Pkcs1v15Sign::new::<Sha256>();
    private_key
        .sign(scheme, &hash)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("alipay sign")))
}

fn verify_data(data: &[u8], sig_bytes: &[u8], public_key: &rsa::RsaPublicKey) -> AppResult<()> {
    let hash = Sha256::digest(data);
    let scheme = Pkcs1v15Sign::new::<Sha256>();
    public_key.verify(scheme, &hash, sig_bytes).map_err(|e| {
        tracing::warn!("alipay signature verification failed: {e}");
        AppError::BadRequest("alipay callback signature mismatch".into())
    })
}

fn build_signing_string(params: &BTreeMap<String, String>) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn sign_params(
    params: &BTreeMap<String, String>,
    private_key: &rsa::RsaPrivateKey,
) -> AppResult<String> {
    let signing_str = build_signing_string(params);
    let sig = sign_data(signing_str.as_bytes(), private_key)?;
    Ok(BASE64.encode(sig))
}

fn verify_signature(
    params: &BTreeMap<String, String>,
    signature_b64: &str,
    public_key: &rsa::RsaPublicKey,
) -> AppResult<()> {
    let sig_bytes = BASE64.decode(signature_b64).map_err(|e| {
        AppError::Internal(anyhow::Error::from(e).context("alipay signature base64 decode"))
    })?;
    let signing_str = build_signing_string(params);
    verify_data(signing_str.as_bytes(), &sig_bytes, public_key)
}

fn cents_to_amount(cents: i64) -> String {
    format!("{:.2}", cents as f64 / 100.0)
}

fn trade_status_to_payment(status: &str) -> PaymentStatus {
    match status {
        "TRADE_SUCCESS" | "TRADE_FINISHED" => PaymentStatus::Paid,
        "TRADE_CLOSED" => PaymentStatus::Cancelled,
        "WAIT_BUYER_PAY" => PaymentStatus::Pending,
        _ => PaymentStatus::Pending,
    }
}

fn alipay_error(code: &str, msg: &str) -> AppError {
    AppError::Internal(anyhow::anyhow!("alipay api error: {code} - {msg}"))
}

fn build_common_params(
    creds: &AlipayCredentials,
    method: &str,
    biz_content: &str,
    notify_url: Option<&str>,
) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    params.insert("app_id".to_string(), creds.app_id.clone());
    params.insert("method".to_string(), method.to_string());
    params.insert("charset".to_string(), CHARSET.to_string());
    params.insert("sign_type".to_string(), SIGN_TYPE.to_string());
    params.insert(
        "timestamp".to_string(),
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    );
    params.insert("version".to_string(), VERSION.to_string());
    params.insert("biz_content".to_string(), biz_content.to_string());
    if let Some(url) = notify_url {
        params.insert("notify_url".to_string(), url.to_string());
    }
    params
}

async fn call_alipay(
    creds: &AlipayCredentials,
    private_key: &rsa::RsaPrivateKey,
    method: &str,
    biz_content: &str,
    notify_url: Option<&str>,
) -> AppResult<String> {
    let params = build_common_params(creds, method, biz_content, notify_url);
    let sign = sign_params(&params, private_key)?;

    let mut form = params.clone();
    form.insert("sign".to_string(), sign);

    let client = super::http_client();
    let resp = client
        .post(gateway_url(creds))
        .form(&form)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("alipay request")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(anyhow::anyhow!(
            "alipay http error: {status} - {text}"
        )));
    }

    resp.text()
        .await
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("alipay read body")))
}

fn extract_alipay_public_key(channel: &PaymentChannel) -> AppResult<String> {
    channel
        .webhook_secret
        .as_deref()
        .ok_or_else(|| {
            AppError::BadRequest("alipay: alipay_public_key (webhook_secret) not configured".into())
        })
        .map(String::from)
}

pub struct AlipayProvider {
    encrypt_key: [u8; 32],
}

impl AlipayProvider {
    pub fn new(encrypt_key: [u8; 32]) -> Self {
        Self { encrypt_key }
    }
}

#[async_trait::async_trait]
impl PaymentProvider for AlipayProvider {
    fn name(&self) -> &str {
        "alipay"
    }

    async fn create(
        &self,
        channel: &PaymentChannel,
        order: &PaymentOrder,
        _return_url: Option<&str>,
        notify_url: Option<&str>,
    ) -> AppResult<ProviderResponse> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        tracing::info!(
            "alipay create: app_id={}, test_mode={}, gateway={}",
            creds.app_id,
            creds.test_mode,
            gateway_url(&creds)
        );
        let private_key = parse_private_key(&creds.private_key)?;

        let biz = PrecreateBizContent {
            out_trade_no: order.id.to_string(),
            total_amount: cents_to_amount(order.amount),
            subject: order.title.clone(),
            timeout_express: Some("30m".to_string()),
        };
        let biz_json = serde_json::to_string(&biz).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("biz_content serialize"))
        })?;

        let body = call_alipay(
            &creds,
            &private_key,
            "alipay.trade.precreate",
            &biz_json,
            notify_url,
        )
        .await?;

        let resp: AlipayResponse<PrecreateResult> = serde_json::from_str(&body).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("alipay response parse"))
        })?;

        let result = resp.precreate.ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("alipay: missing precreate response"))
        })?;

        if result.code.as_deref() != Some("10000") {
            return Err(alipay_error(
                result.sub_code.as_deref().unwrap_or("unknown"),
                result.sub_msg.as_deref().unwrap_or("unknown"),
            ));
        }

        Ok(ProviderResponse {
            provider_order_id: result
                .trade_no
                .or(result.out_trade_no)
                .unwrap_or(order.id.to_string()),
            redirect_url: None,
            qr_code: result.qr_code,
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

        let biz = QueryBizContent {
            out_trade_no: provider_order_id.to_string(),
        };
        let biz_json = serde_json::to_string(&biz).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("biz_content serialize"))
        })?;

        let body = call_alipay(&creds, &private_key, "alipay.trade.query", &biz_json, None).await?;

        let resp: AlipayResponse<QueryResult> = serde_json::from_str(&body).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("alipay response parse"))
        })?;

        let result = resp
            .query
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("alipay: missing query response")))?;

        if result.code.as_deref() != Some("10000") {
            return Ok(ProviderStatus {
                status: PaymentStatus::Pending,
                provider_tx_id: None,
                paid_at: None,
                amount: None,
            });
        }

        let status = result
            .trade_status
            .as_deref()
            .map(trade_status_to_payment)
            .unwrap_or(PaymentStatus::Pending);

        Ok(ProviderStatus {
            status,
            provider_tx_id: result.trade_no,
            paid_at: result.send_pay_date,
            amount: None,
        })
    }

    async fn cancel(&self, channel: &PaymentChannel, provider_order_id: &str) -> AppResult<()> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let private_key = parse_private_key(&creds.private_key)?;

        let biz = CancelBizContent {
            out_trade_no: provider_order_id.to_string(),
        };
        let biz_json = serde_json::to_string(&biz).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("biz_content serialize"))
        })?;

        let body =
            call_alipay(&creds, &private_key, "alipay.trade.cancel", &biz_json, None).await?;

        let resp: AlipayResponse<CancelResult> = serde_json::from_str(&body).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("alipay response parse"))
        })?;

        let result = resp.cancel.ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("alipay: missing cancel response"))
        })?;

        if result.code.as_deref() != Some("10000") {
            return Err(alipay_error("cancel_failed", "cancel returned non-10000"));
        }

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
        let biz = RefundBizContent {
            out_trade_no: provider_order_id.to_string(),
            refund_amount: cents_to_amount(amount),
            out_request_no: Some(refund_no.clone()),
            refund_reason: reason.map(String::from),
        };
        let biz_json = serde_json::to_string(&biz).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("biz_content serialize"))
        })?;

        let body =
            call_alipay(&creds, &private_key, "alipay.trade.refund", &biz_json, None).await?;

        let resp: AlipayResponse<RefundResult> = serde_json::from_str(&body).map_err(|e| {
            AppError::Internal(anyhow::Error::from(e).context("alipay response parse"))
        })?;

        let result = resp.refund.ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("alipay: missing refund response"))
        })?;

        if result.code.as_deref() != Some("10000") {
            return Err(alipay_error(
                result.sub_code.as_deref().unwrap_or("unknown"),
                result.sub_msg.as_deref().unwrap_or("unknown"),
            ));
        }

        Ok(RefundResponse {
            provider_refund_id: result.trade_no.unwrap_or(refund_no),
        })
    }

    async fn verify_callback(
        &self,
        channel: &PaymentChannel,
        _headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<CallbackData> {
        let body_str = std::str::from_utf8(body)
            .map_err(|e| AppError::BadRequest(format!("alipay callback: invalid utf8: {e}")))?;

        let params: BTreeMap<String, String> = serde_urlencoded::from_str(body_str)
            .map_err(|e| AppError::BadRequest(format!("alipay callback parse: {e}")))?;

        let signature = params
            .get("sign")
            .ok_or_else(|| AppError::BadRequest("alipay callback: missing sign".into()))?;

        let alipay_pub_key_str = extract_alipay_public_key(channel)?;
        let alipay_pub_key = parse_public_key(&alipay_pub_key_str)?;

        let mut check_params = params.clone();
        check_params.remove("sign");
        check_params.remove("sign_type");

        verify_signature(&check_params, signature, &alipay_pub_key)?;

        let trade_status = params
            .get("trade_status")
            .map(String::as_str)
            .unwrap_or("UNKNOWN");
        let status = trade_status_to_payment(trade_status);

        let amount = params
            .get("total_amount")
            .and_then(|v| {
                let f: f64 = v.parse().ok()?;
                Some((f * 100.0) as i64)
            })
            .unwrap_or(0);

        let trade_no = params.get("trade_no").cloned().unwrap_or_default();
        let provider_tx_id = if trade_no.is_empty() {
            None
        } else {
            Some(trade_no)
        };

        Ok(CallbackData {
            provider_order_id: params.get("out_trade_no").cloned().unwrap_or_default(),
            status,
            amount,
            provider_tx_id,
            paid_at: params.get("gmt_payment").cloned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::{EncodePublicKey, LineEnding};

    fn generate_keypair() -> (rsa::RsaPrivateKey, rsa::RsaPublicKey) {
        let mut rng = rsa::rand_core::OsRng;
        let private = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = rsa::RsaPublicKey::from(&private);
        (private, public)
    }

    #[test]
    fn cents_to_amount_works() {
        assert_eq!(cents_to_amount(0), "0.00");
        assert_eq!(cents_to_amount(1), "0.01");
        assert_eq!(cents_to_amount(999), "9.99");
        assert_eq!(cents_to_amount(100), "1.00");
        assert_eq!(cents_to_amount(12345), "123.45");
    }

    #[test]
    fn trade_status_mapping() {
        assert!(matches!(
            trade_status_to_payment("TRADE_SUCCESS"),
            PaymentStatus::Paid
        ));
        assert!(matches!(
            trade_status_to_payment("TRADE_FINISHED"),
            PaymentStatus::Paid
        ));
        assert!(matches!(
            trade_status_to_payment("TRADE_CLOSED"),
            PaymentStatus::Cancelled
        ));
        assert!(matches!(
            trade_status_to_payment("WAIT_BUYER_PAY"),
            PaymentStatus::Pending
        ));
        assert!(matches!(
            trade_status_to_payment("UNKNOWN"),
            PaymentStatus::Pending
        ));
    }

    #[test]
    fn gateway_url_selection() {
        let live = AlipayCredentials {
            app_id: "123".into(),
            private_key: String::new(),
            test_mode: false,
        };
        assert_eq!(gateway_url(&live), GATEWAY_LIVE);

        let test = AlipayCredentials {
            app_id: "123".into(),
            private_key: String::new(),
            test_mode: true,
        };
        assert_eq!(gateway_url(&test), GATEWAY_TEST);
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let (private_key, public_key) = generate_keypair();

        let mut params = BTreeMap::new();
        params.insert("app_id".to_string(), "2021001234".to_string());
        params.insert(
            "biz_content".to_string(),
            r#"{"out_trade_no":"test"}"#.to_string(),
        );
        params.insert("method".to_string(), "alipay.trade.precreate".to_string());

        let signature = sign_params(&params, &private_key).unwrap();
        verify_signature(&params, &signature, &public_key).unwrap();
    }

    #[test]
    fn verify_rejects_tampered_params() {
        let (private_key, public_key) = generate_keypair();

        let mut params = BTreeMap::new();
        params.insert("app_id".to_string(), "2021001234".to_string());
        params.insert("amount".to_string(), "9.99".to_string());

        let signature = sign_params(&params, &private_key).unwrap();

        let mut tampered = params.clone();
        tampered.insert("amount".to_string(), "0.01".to_string());

        assert!(verify_signature(&tampered, &signature, &public_key).is_err());
    }

    #[test]
    fn build_common_params_includes_required_fields() {
        let creds = AlipayCredentials {
            app_id: "2021001234".to_string(),
            private_key: String::new(),
            test_mode: false,
        };
        let params = build_common_params(&creds, "alipay.trade.precreate", "{}", None);
        assert_eq!(params.get("app_id").unwrap(), "2021001234");
        assert_eq!(params.get("method").unwrap(), "alipay.trade.precreate");
        assert_eq!(params.get("charset").unwrap(), "utf-8");
        assert_eq!(params.get("sign_type").unwrap(), "RSA2");
        assert_eq!(params.get("version").unwrap(), "1.0");
        assert!(params.get("timestamp").is_some());
        assert_eq!(params.get("biz_content").unwrap(), "{}");
    }

    #[test]
    fn sign_params_produces_base64() {
        let (private_key, _) = generate_keypair();

        let mut params = BTreeMap::new();
        params.insert("a".to_string(), "1".to_string());
        let sig = sign_params(&params, &private_key).unwrap();

        assert!(BASE64.decode(&sig).is_ok());
    }

    #[tokio::test]
    async fn verify_callback_parses_trade_success() {
        let (private_key, public_key) = generate_keypair();

        let key = [42u8; 32];
        let pub_key_pem = public_key.to_public_key_pem(LineEnding::LF).unwrap();

        let mut params = BTreeMap::new();
        params.insert("app_id".to_string(), "2021001234".to_string());
        params.insert("out_trade_no".to_string(), "order_doc_001".to_string());
        params.insert("trade_no".to_string(), "2024010100001".to_string());
        params.insert("trade_status".to_string(), "TRADE_SUCCESS".to_string());
        params.insert("total_amount".to_string(), "99.99".to_string());
        params.insert("gmt_payment".to_string(), "2024-01-01 12:00:00".to_string());

        let signature = sign_params(&params, &private_key).unwrap();
        params.insert("sign".to_string(), signature);
        params.insert("sign_type".to_string(), "RSA2".to_string());

        let body = serde_urlencoded::to_string(&params).unwrap();

        let channel = PaymentChannel {
            id: crate::types::snowflake_id::SnowflakeId(1),
            tenant_id: None,
            provider: "alipay".into(),
            name: "Alipay".into(),
            is_live: 0,
            credentials: String::new(),
            webhook_secret: Some(pub_key_pem),
            settings: None,
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: crate::utils::tz::Timestamp::default(),
            updated_at: crate::utils::tz::Timestamp::default(),
        };

        let provider = AlipayProvider::new(key);
        let result = provider
            .verify_callback(&channel, &HeaderMap::new(), body.as_bytes())
            .await
            .unwrap();

        assert_eq!(result.provider_order_id, "order_doc_001");
        assert!(matches!(result.status, PaymentStatus::Paid));
        assert_eq!(result.amount, 9999);
        assert_eq!(result.provider_tx_id.as_deref(), Some("2024010100001"));
        assert_eq!(result.paid_at.as_deref(), Some("2024-01-01 12:00:00"));
    }
}

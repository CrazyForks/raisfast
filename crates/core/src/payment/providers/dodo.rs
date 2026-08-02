use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::errors::app_error::{AppError, AppResult};
use crate::models::payment_channel::PaymentChannel;
use crate::models::payment_order::{PaymentOrder, PaymentStatus};
use crate::payment::PaymentProvider;
use crate::payment::crypto::aes256gcm_decrypt;
use crate::payment::provider::{CallbackData, ProviderResponse, ProviderStatus, RefundResponse};

type HmacSha256 = Hmac<Sha256>;

const BASE_URL_LIVE: &str = "https://live.dodopayments.com";
const BASE_URL_TEST: &str = "https://test.dodopayments.com";

const WEBHOOK_TOLERANCE_SECS: u64 = 300;

#[derive(Deserialize)]
struct DodoCredentials {
    api_key: String,
    #[serde(default)]
    test_mode: bool,
}

fn decrypt_credentials(
    channel: &PaymentChannel,
    encrypt_key: &[u8; 32],
) -> AppResult<DodoCredentials> {
    let decrypted = aes256gcm_decrypt(&channel.credentials, encrypt_key)?;
    serde_json::from_str(&decrypted)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("dodo credentials parse")))
}

fn base_url(creds: &DodoCredentials) -> &str {
    if creds.test_mode {
        BASE_URL_TEST
    } else {
        BASE_URL_LIVE
    }
}

fn dodo_status_to_payment(status: &str) -> PaymentStatus {
    match status {
        "succeeded" => PaymentStatus::Paid,
        "failed" => PaymentStatus::Failed,
        "cancelled" => PaymentStatus::Cancelled,
        "disputed" => PaymentStatus::Disputed,
        _ => PaymentStatus::Pending,
    }
}

fn dodo_error(e: reqwest::Error) -> AppError {
    AppError::Internal(anyhow::Error::from(e).context("dodo api"))
}

fn extract_product_id(channel: &PaymentChannel) -> AppResult<String> {
    channel
        .settings
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("product_id").cloned())
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| {
            AppError::BadRequest("dodo: product_id not found in channel settings".into())
        })
}

fn decrypt_webhook_secret(channel: &PaymentChannel, encrypt_key: &[u8; 32]) -> AppResult<String> {
    let encrypted = channel
        .webhook_secret
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("dodo webhook_secret not configured".into()))?;
    aes256gcm_decrypt(encrypted, encrypt_key)
}

#[derive(Serialize)]
struct CheckoutRequest {
    product_cart: Vec<ProductCartItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    customer: Option<CustomerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    return_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ProductCartItem {
    product_id: String,
    quantity: i32,
}

#[derive(Serialize)]
struct CustomerInfo {
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Deserialize)]
struct CheckoutResponse {
    payment_id: String,
    checkout_url: String,
    #[allow(dead_code)]
    client_secret: Option<String>,
}

#[derive(Deserialize)]
struct PaymentResponse {
    #[allow(dead_code)]
    payment_id: String,
    status: String,
    total_amount: Option<i64>,
    #[allow(dead_code)]
    currency: Option<String>,
}

#[derive(Serialize)]
struct RefundRequest {
    payment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct RefundApiResponse {
    #[allow(dead_code)]
    refund_id: Option<String>,
}

#[derive(Deserialize)]
struct DodoWebhookPayload {
    #[allow(dead_code)]
    business_id: Option<String>,
    #[serde(rename = "type")]
    event_type: Option<String>,
    #[allow(dead_code)]
    timestamp: Option<String>,
    data: Option<WebhookData>,
}

#[derive(Deserialize)]
struct WebhookData {
    #[allow(dead_code)]
    payload_type: Option<String>,
    payment_id: Option<String>,
    #[allow(dead_code)]
    status: Option<String>,
    total_amount: Option<i64>,
    #[allow(dead_code)]
    currency: Option<String>,
    #[allow(dead_code)]
    customer: Option<WebhookCustomer>,
}

#[derive(Deserialize)]
struct WebhookCustomer {
    #[allow(dead_code)]
    customer_id: Option<String>,
    #[allow(dead_code)]
    name: Option<String>,
    #[allow(dead_code)]
    email: Option<String>,
}

async fn do_create_checkout(
    creds: &DodoCredentials,
    product_id: &str,
    quantity: i32,
    customer_email: Option<&str>,
    customer_name: Option<&str>,
    return_url: Option<&str>,
    metadata: serde_json::Value,
) -> AppResult<CheckoutResponse> {
    let url = format!("{}/checkouts", base_url(creds));
    let customer = customer_email.map(|email| CustomerInfo {
        email: email.to_string(),
        name: customer_name.map(String::from),
    });

    let body = CheckoutRequest {
        product_cart: vec![ProductCartItem {
            product_id: product_id.to_string(),
            quantity,
        }],
        customer,
        return_url: return_url.map(String::from),
        metadata: Some(metadata),
    };

    let client = super::http_client();
    let resp = client
        .post(&url)
        .bearer_auth(&creds.api_key)
        .json(&body)
        .send()
        .await
        .map_err(dodo_error)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(anyhow::anyhow!(
            "dodo create checkout failed: {status} - {text}"
        )));
    }

    resp.json::<CheckoutResponse>().await.map_err(dodo_error)
}

async fn do_get_payment(creds: &DodoCredentials, payment_id: &str) -> AppResult<PaymentResponse> {
    let url = format!("{}/payments/{payment_id}", base_url(creds));
    let client = super::http_client();
    let resp = client
        .get(&url)
        .bearer_auth(&creds.api_key)
        .send()
        .await
        .map_err(dodo_error)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(anyhow::anyhow!(
            "dodo get payment failed: {status} - {text}"
        )));
    }

    resp.json::<PaymentResponse>().await.map_err(dodo_error)
}

async fn do_create_refund(
    creds: &DodoCredentials,
    payment_id: &str,
    reason: Option<&str>,
) -> AppResult<RefundApiResponse> {
    let url = format!("{}/refunds", base_url(creds));

    let body = RefundRequest {
        payment_id: payment_id.to_string(),
        reason: reason.map(String::from),
    };

    let client = super::http_client();
    let resp = client
        .post(&url)
        .bearer_auth(&creds.api_key)
        .json(&body)
        .send()
        .await
        .map_err(dodo_error)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(anyhow::anyhow!(
            "dodo create refund failed: {status} - {text}"
        )));
    }

    resp.json::<RefundApiResponse>().await.map_err(dodo_error)
}

fn verify_webhook_signature(
    body: &[u8],
    webhook_id: &str,
    signature: &str,
    timestamp: &str,
    webhook_secret: &str,
) -> AppResult<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("system time")))?
        .as_secs();

    let ts: u64 = timestamp
        .parse()
        .map_err(|e| AppError::BadRequest(format!("dodo webhook: invalid timestamp: {e}")))?;

    if now.abs_diff(ts) > WEBHOOK_TOLERANCE_SECS {
        return Err(AppError::BadRequest(
            "dodo webhook: timestamp expired".into(),
        ));
    }

    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("hmac init")))?;
    mac.update(webhook_id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);

    let expected = BASE64.encode(mac.finalize().into_bytes());

    let signatures: Vec<&str> = signature.split(' ').collect();
    if signatures
        .iter()
        .all(|s| !s.eq_ignore_ascii_case(&expected))
    {
        return Err(AppError::BadRequest(
            "dodo webhook: signature mismatch".into(),
        ));
    }

    Ok(())
}

pub struct DodoProvider {
    encrypt_key: [u8; 32],
}

impl DodoProvider {
    pub fn new(encrypt_key: [u8; 32]) -> Self {
        Self { encrypt_key }
    }
}

#[async_trait::async_trait]
impl PaymentProvider for DodoProvider {
    fn name(&self) -> &str {
        "dodo"
    }

    async fn create(
        &self,
        channel: &PaymentChannel,
        order: &PaymentOrder,
        return_url: Option<&str>,
        _notify_url: Option<&str>,
    ) -> AppResult<ProviderResponse> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let product_id = extract_product_id(channel)?;

        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "payment_order_id".to_string(),
            serde_json::Value::String(order.id.to_string()),
        );
        if let Some(ref order_id) = order.order_id {
            metadata.insert(
                "order_id".to_string(),
                serde_json::Value::String(order_id.clone()),
            );
        }

        let checkout = do_create_checkout(
            &creds,
            &product_id,
            1,
            None,
            None,
            return_url,
            serde_json::Value::Object(metadata),
        )
        .await?;

        Ok(ProviderResponse {
            provider_order_id: checkout.payment_id,
            redirect_url: Some(checkout.checkout_url),
            qr_code: None,
            client_secret: checkout.client_secret,
        })
    }

    async fn query(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
    ) -> AppResult<ProviderStatus> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let payment = do_get_payment(&creds, provider_order_id).await?;

        Ok(ProviderStatus {
            status: dodo_status_to_payment(&payment.status),
            provider_tx_id: None,
            paid_at: None,
            amount: payment.total_amount,
        })
    }

    async fn cancel(&self, _channel: &PaymentChannel, _provider_order_id: &str) -> AppResult<()> {
        Ok(())
    }

    async fn refund(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
        _amount: i64,
        reason: Option<&str>,
    ) -> AppResult<RefundResponse> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let resp = do_create_refund(&creds, provider_order_id, reason).await?;

        Ok(RefundResponse {
            provider_refund_id: resp
                .refund_id
                .unwrap_or_else(|| format!("dodo_refund_{provider_order_id}")),
        })
    }

    async fn verify_callback(
        &self,
        channel: &PaymentChannel,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<CallbackData> {
        let webhook_id = headers
            .get("webhook-id")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("missing webhook-id header".into()))?;

        let signature = headers
            .get("webhook-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("missing webhook-signature header".into()))?;

        let timestamp = headers
            .get("webhook-timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("missing webhook-timestamp header".into()))?;

        let webhook_secret = decrypt_webhook_secret(channel, &self.encrypt_key)?;

        verify_webhook_signature(body, webhook_id, signature, timestamp, &webhook_secret)?;

        let payload: DodoWebhookPayload = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("dodo webhook parse: {e}")))?;

        let event_type = payload.event_type.as_deref().unwrap_or("unknown");
        let data = payload
            .data
            .ok_or_else(|| AppError::BadRequest("dodo webhook: missing data field".into()))?;

        let (provider_order_id, status, amount) = match event_type {
            "payment.succeeded" => (
                data.payment_id.unwrap_or_default(),
                PaymentStatus::Paid,
                data.total_amount.unwrap_or(0),
            ),
            "payment.failed" => (
                data.payment_id.unwrap_or_default(),
                PaymentStatus::Failed,
                data.total_amount.unwrap_or(0),
            ),
            "payment.cancelled" => (
                data.payment_id.unwrap_or_default(),
                PaymentStatus::Cancelled,
                data.total_amount.unwrap_or(0),
            ),
            "refund.succeeded" => (
                data.payment_id.unwrap_or_default(),
                PaymentStatus::Refunded,
                data.total_amount.unwrap_or(0),
            ),
            "refund.failed" => (
                data.payment_id.unwrap_or_default(),
                PaymentStatus::Pending,
                data.total_amount.unwrap_or(0),
            ),
            _ => {
                return Err(AppError::BadRequest(format!(
                    "dodo: unhandled event type: {event_type}"
                )));
            }
        };

        Ok(CallbackData {
            provider_order_id,
            status,
            amount,
            provider_tx_id: None,
            paid_at: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [42u8; 32]
    }

    #[test]
    fn dodo_status_mapping() {
        assert!(matches!(
            dodo_status_to_payment("succeeded"),
            PaymentStatus::Paid
        ));
        assert!(matches!(
            dodo_status_to_payment("failed"),
            PaymentStatus::Failed
        ));
        assert!(matches!(
            dodo_status_to_payment("cancelled"),
            PaymentStatus::Cancelled
        ));
        assert!(matches!(
            dodo_status_to_payment("disputed"),
            PaymentStatus::Disputed
        ));
        assert!(matches!(
            dodo_status_to_payment("pending"),
            PaymentStatus::Pending
        ));
    }

    #[test]
    fn base_url_selection() {
        let live = DodoCredentials {
            api_key: "key".into(),
            test_mode: false,
        };
        assert_eq!(base_url(&live), BASE_URL_LIVE);

        let test = DodoCredentials {
            api_key: "key".into(),
            test_mode: true,
        };
        assert_eq!(base_url(&test), BASE_URL_TEST);
    }

    #[test]
    fn extract_product_id_from_settings() {
        let key = test_key();
        let encrypted =
            crate::payment::crypto::aes256gcm_encrypt(r#"{"api_key":"test_key"}"#, &key).unwrap();

        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "dodo".into(),
            name: "Dodo".into(),
            is_live: 0,
            credentials: encrypted,
            webhook_secret: None,
            settings: Some(r#"{"product_id":"prod_abc123"}"#.into()),
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: crate::utils::tz::Timestamp::default(),
            updated_at: crate::utils::tz::Timestamp::default(),
        };

        let pid = extract_product_id(&channel).unwrap();
        assert_eq!(pid, "prod_abc123");
    }

    #[test]
    fn extract_product_id_missing_settings() {
        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "dodo".into(),
            name: "Dodo".into(),
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

        assert!(extract_product_id(&channel).is_err());
    }

    #[test]
    fn decrypt_credentials_works() {
        let key = test_key();
        let encrypted = crate::payment::crypto::aes256gcm_encrypt(
            r#"{"api_key":"sk_test_123","test_mode":true}"#,
            &key,
        )
        .unwrap();

        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "dodo".into(),
            name: "Dodo".into(),
            is_live: 0,
            credentials: encrypted,
            webhook_secret: None,
            settings: None,
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: crate::utils::tz::Timestamp::default(),
            updated_at: crate::utils::tz::Timestamp::default(),
        };

        let creds = decrypt_credentials(&channel, &key).unwrap();
        assert_eq!(creds.api_key, "sk_test_123");
        assert!(creds.test_mode);
    }

    #[test]
    fn webhook_signature_verification_valid() {
        let webhook_secret = "whsec_test123";
        let webhook_id = "msg_xxx";
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let body = br#"{"type":"payment.succeeded","data":{}}"#;

        let timestamp_str = timestamp.to_string();
        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(webhook_id.as_bytes());
        mac.update(b".");
        mac.update(timestamp_str.as_bytes());
        mac.update(b".");
        mac.update(body);
        let sig = BASE64.encode(mac.finalize().into_bytes());

        assert!(
            verify_webhook_signature(body, webhook_id, &sig, &timestamp_str, webhook_secret,)
                .is_ok()
        );
    }

    #[test]
    fn webhook_signature_verification_invalid() {
        let body = br#"{"type":"test"}"#;
        assert!(verify_webhook_signature(body, "msg_1", "badsig", "9999999999", "secret").is_err());
    }

    #[test]
    fn webhook_signature_expired_timestamp() {
        let webhook_secret = "whsec_test";
        let webhook_id = "msg_xxx";
        let old_timestamp = 1_000_000_000u64;
        let body = br#"{"type":"payment.succeeded"}"#;

        let old_ts_str = old_timestamp.to_string();
        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(webhook_id.as_bytes());
        mac.update(b".");
        mac.update(old_ts_str.as_bytes());
        mac.update(b".");
        mac.update(body);
        let sig = BASE64.encode(mac.finalize().into_bytes());

        assert!(
            verify_webhook_signature(body, webhook_id, &sig, &old_ts_str, webhook_secret,).is_err()
        );
    }

    #[tokio::test]
    async fn verify_callback_parses_payment_succeeded() {
        let key = test_key();
        let webhook_secret = "wh_secret_123";
        let encrypted_secret =
            crate::payment::crypto::aes256gcm_encrypt(webhook_secret, &key).unwrap();

        let body = serde_json::json!({
            "type": "payment.succeeded",
            "data": {
                "payload_type": "Payment",
                "payment_id": "pay_001",
                "status": "succeeded",
                "total_amount": 2999
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let webhook_id = "msg_test";
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let timestamp_str = timestamp.to_string();
        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(webhook_id.as_bytes());
        mac.update(b".");
        mac.update(timestamp_str.as_bytes());
        mac.update(b".");
        mac.update(&body_bytes);
        let sig = BASE64.encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("webhook-id", webhook_id.parse().unwrap());
        headers.insert("webhook-signature", sig.parse().unwrap());
        headers.insert("webhook-timestamp", timestamp_str.parse().unwrap());

        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "dodo".into(),
            name: "Dodo".into(),
            is_live: 0,
            credentials: String::new(),
            webhook_secret: Some(encrypted_secret),
            settings: None,
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: crate::utils::tz::Timestamp::default(),
            updated_at: crate::utils::tz::Timestamp::default(),
        };

        let provider = DodoProvider::new(key);
        let result = provider
            .verify_callback(&channel, &headers, &body_bytes)
            .await
            .unwrap();

        assert_eq!(result.provider_order_id, "pay_001");
        assert!(matches!(result.status, PaymentStatus::Paid));
        assert_eq!(result.amount, 2999);
    }

    #[tokio::test]
    async fn verify_callback_parses_payment_failed() {
        let key = test_key();
        let webhook_secret = "wh_secret_fail";
        let encrypted_secret =
            crate::payment::crypto::aes256gcm_encrypt(webhook_secret, &key).unwrap();

        let body = serde_json::json!({
            "type": "payment.failed",
            "data": {
                "payload_type": "Payment",
                "payment_id": "pay_002",
                "status": "failed",
                "total_amount": 0
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let webhook_id = "msg_fail";
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let timestamp_str = timestamp.to_string();
        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(webhook_id.as_bytes());
        mac.update(b".");
        mac.update(timestamp_str.as_bytes());
        mac.update(b".");
        mac.update(&body_bytes);
        let sig = BASE64.encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("webhook-id", webhook_id.parse().unwrap());
        headers.insert("webhook-signature", sig.parse().unwrap());
        headers.insert("webhook-timestamp", timestamp_str.parse().unwrap());

        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "dodo".into(),
            name: "Dodo".into(),
            is_live: 0,
            credentials: String::new(),
            webhook_secret: Some(encrypted_secret),
            settings: None,
            is_active: 1,
            sort_order: 0,
            version: 1,
            created_at: crate::utils::tz::Timestamp::default(),
            updated_at: crate::utils::tz::Timestamp::default(),
        };

        let provider = DodoProvider::new(key);
        let result = provider
            .verify_callback(&channel, &headers, &body_bytes)
            .await
            .unwrap();

        assert_eq!(result.provider_order_id, "pay_002");
        assert!(matches!(result.status, PaymentStatus::Failed));
    }

    #[tokio::test]
    async fn cancel_is_noop() {
        let key = test_key();
        let channel = PaymentChannel {
            id: 1,
            tenant_id: None,
            provider: "dodo".into(),
            name: "Dodo".into(),
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

        let provider = DodoProvider::new(key);
        provider.cancel(&channel, "any_id").await.unwrap();
    }
}

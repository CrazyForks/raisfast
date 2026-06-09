use axum::http::HeaderMap;
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

const BASE_URL_LIVE: &str = "https://api.creem.io/v1";
const BASE_URL_TEST: &str = "https://test-api.creem.io/v1";

#[derive(Deserialize)]
struct CreemCredentials {
    api_key: String,
    #[serde(default)]
    test_mode: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct CheckoutRequest {
    product_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    success_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CheckoutResponse {
    id: String,
    checkout_url: String,
}

#[derive(Deserialize)]
struct CheckoutDetailResponse {
    #[allow(dead_code)]
    id: String,
    order: Option<CreemOrderDetail>,
}

#[derive(Deserialize)]
struct CreemOrderDetail {
    status: Option<String>,
    #[allow(dead_code)]
    amount: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreemWebhookPayload {
    event_type: Option<String>,
    object: serde_json::Value,
}

fn decrypt_credentials(
    channel: &PaymentChannel,
    encrypt_key: &[u8; 32],
) -> AppResult<CreemCredentials> {
    let decrypted = aes256gcm_decrypt(&channel.credentials, encrypt_key)?;
    serde_json::from_str(&decrypted)
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("creem credentials parse")))
}

fn base_url(creds: &CreemCredentials) -> &str {
    if creds.test_mode {
        BASE_URL_TEST
    } else {
        BASE_URL_LIVE
    }
}

fn creem_status_to_payment(status: &str) -> PaymentStatus {
    match status {
        "paid" | "complete" | "completed" => PaymentStatus::Paid,
        "cancelled" | "canceled" => PaymentStatus::Cancelled,
        "expired" => PaymentStatus::Expired,
        "refunded" => PaymentStatus::Refunded,
        _ => PaymentStatus::Pending,
    }
}

fn creem_error(e: reqwest::Error) -> AppError {
    AppError::Internal(anyhow::Error::from(e).context("creem api"))
}

async fn do_create_checkout(
    creds: &CreemCredentials,
    product_id: &str,
    return_url: Option<&str>,
    metadata: serde_json::Value,
) -> AppResult<CheckoutResponse> {
    let url = format!("{}/checkouts", base_url(creds));
    let mut body = CheckoutRequest {
        product_id: product_id.to_string(),
        success_url: None,
        metadata: Some(metadata),
    };
    if let Some(url) = return_url {
        body.success_url = Some(url.to_string());
    }

    let client = super::http_client();
    let resp = client
        .post(&url)
        .header("x-api-key", &creds.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("creem create checkout request failed: {e}");
            creem_error(e)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        tracing::error!("creem create checkout rejected: status={status}, body={text}");
        return Err(AppError::Internal(anyhow::anyhow!(
            "creem create checkout failed: {status} - {text}"
        )));
    }

    let resp_text = resp.text().await.map_err(|e| {
        tracing::error!("creem create checkout read body failed: {e}");
        creem_error(e)
    })?;
    let checkout: CheckoutResponse = serde_json::from_str(&resp_text).map_err(|e| {
        tracing::error!("creem create checkout parse failed: {e}, body={resp_text}");
        AppError::Internal(anyhow::anyhow!("creem response parse failed: {e}"))
    })?;
    Ok(checkout)
}

async fn do_get_checkout(
    creds: &CreemCredentials,
    checkout_id: &str,
) -> AppResult<CheckoutDetailResponse> {
    let url = format!("{}/checkouts/{checkout_id}", base_url(creds));
    let client = super::http_client();
    let resp = client
        .get(&url)
        .header("x-api-key", &creds.api_key)
        .send()
        .await
        .map_err(creem_error)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(anyhow::anyhow!(
            "creem get checkout failed: {status} - {text}"
        )));
    }

    resp.json::<CheckoutDetailResponse>()
        .await
        .map_err(creem_error)
}

fn verify_signature(body: &[u8], signature: &str, webhook_secret: &str) -> AppResult<()> {
    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())
        .map_err(|e| AppError::Internal(anyhow::Error::from(e).context("hmac init")))?;
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());
    if !expected.eq_ignore_ascii_case(signature) {
        return Err(AppError::BadRequest("creem signature mismatch".into()));
    }
    Ok(())
}

fn extract_product_id(channel: &PaymentChannel) -> AppResult<String> {
    channel
        .settings
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("product_id").cloned())
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| {
            AppError::BadRequest("creem: product_id not found in channel settings".into())
        })
}

pub struct CreemProvider {
    encrypt_key: [u8; 32],
}

impl CreemProvider {
    pub fn new(encrypt_key: [u8; 32]) -> Self {
        Self { encrypt_key }
    }
}

#[async_trait::async_trait]
impl PaymentProvider for CreemProvider {
    fn name(&self) -> &str {
        "creem"
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
            return_url,
            serde_json::Value::Object(metadata),
        )
        .await?;

        let provider_order_id = checkout.id.clone();

        tracing::info!(
            "creem checkout created: id={}, url={}",
            provider_order_id,
            checkout.checkout_url
        );

        Ok(ProviderResponse {
            provider_order_id,
            redirect_url: Some(checkout.checkout_url),
            qr_code: None,
            client_secret: None,
        })
    }

    async fn query(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
    ) -> AppResult<ProviderStatus> {
        let creds = decrypt_credentials(channel, &self.encrypt_key)?;
        let detail = do_get_checkout(&creds, provider_order_id).await?;

        let status = detail
            .order
            .as_ref()
            .and_then(|o| o.status.as_deref())
            .map(creem_status_to_payment)
            .unwrap_or(PaymentStatus::Pending);

        Ok(ProviderStatus {
            status,
            provider_tx_id: None,
            paid_at: None,
            amount: detail.order.and_then(|o| o.amount),
        })
    }

    async fn cancel(&self, _channel: &PaymentChannel, _provider_order_id: &str) -> AppResult<()> {
        Ok(())
    }

    async fn refund(
        &self,
        _channel: &PaymentChannel,
        _provider_order_id: &str,
        _amount: i64,
        _reason: Option<&str>,
    ) -> AppResult<RefundResponse> {
        Err(AppError::BadRequest(
            "creem refunds must be initiated via the merchant dashboard".into(),
        ))
    }

    async fn verify_callback(
        &self,
        channel: &PaymentChannel,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<CallbackData> {
        let signature = headers
            .get("creem-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("missing creem-signature header".into()))?;

        let encrypted_secret = channel
            .webhook_secret
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("creem webhook_secret not configured".into()))?;

        let webhook_secret =
            crate::payment::crypto::aes256gcm_decrypt(encrypted_secret, &self.encrypt_key)?;

        verify_signature(body, signature, &webhook_secret)?;

        let payload: CreemWebhookPayload = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("creem webhook parse: {e}")))?;

        let event_type = payload.event_type.as_deref().unwrap_or("unknown");

        let obj = &payload.object;

        let (provider_order_id, status, amount, provider_tx_id) = match event_type {
            "checkout.completed" => {
                let checkout_id = obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let order = obj.get("order").ok_or_else(|| {
                    AppError::BadRequest("creem checkout.completed: missing order".into())
                })?;
                let raw_status = order
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("paid");
                let amt = order.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
                let tx_id = obj
                    .get("subscription")
                    .and_then(|s| s.get("last_transaction_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                tracing::info!(
                    "creem checkout.completed: checkout={checkout_id}, status={raw_status}, amount={amt}"
                );
                (checkout_id, creem_status_to_payment(raw_status), amt, tx_id)
            }
            "refund.created" => {
                let refund_id = obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let amt = obj
                    .get("refund_amount")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let checkout_id = obj
                    .get("checkout")
                    .and_then(|c| c.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tx_id = obj
                    .get("transaction")
                    .and_then(|t| t.get("id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                tracing::info!(
                    "creem refund.created: refund={refund_id}, checkout={checkout_id}, amount={amt}"
                );
                (checkout_id.to_string(), PaymentStatus::Refunded, amt, tx_id)
            }
            _ => {
                return Err(AppError::BadRequest(format!(
                    "creem: unhandled event type: {event_type}"
                )));
            }
        };

        Ok(CallbackData {
            provider_order_id,
            status,
            amount,
            provider_tx_id,
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

    fn test_channel_id() -> crate::types::snowflake_id::SnowflakeId {
        crate::types::snowflake_id::SnowflakeId(1)
    }

    #[test]
    fn creem_status_mapping() {
        assert!(matches!(
            creem_status_to_payment("paid"),
            PaymentStatus::Paid
        ));
        assert!(matches!(
            creem_status_to_payment("completed"),
            PaymentStatus::Paid
        ));
        assert!(matches!(
            creem_status_to_payment("cancelled"),
            PaymentStatus::Cancelled
        ));
        assert!(matches!(
            creem_status_to_payment("expired"),
            PaymentStatus::Expired
        ));
        assert!(matches!(
            creem_status_to_payment("refunded"),
            PaymentStatus::Refunded
        ));
        assert!(matches!(
            creem_status_to_payment("pending"),
            PaymentStatus::Pending
        ));
    }

    #[test]
    fn base_url_selection() {
        let live = CreemCredentials {
            api_key: "key".into(),
            test_mode: false,
        };
        assert_eq!(base_url(&live), BASE_URL_LIVE);

        let test = CreemCredentials {
            api_key: "key".into(),
            test_mode: true,
        };
        assert_eq!(base_url(&test), BASE_URL_TEST);
    }

    #[test]
    fn signature_verification_valid() {
        let secret = "test_webhook_secret";
        let body = br#"{"eventType":"checkout.completed","object":{}}"#;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());

        assert!(verify_signature(body, &sig, secret).is_ok());
    }

    #[test]
    fn signature_verification_invalid() {
        let body = br#"{"eventType":"test"}"#;
        assert!(verify_signature(body, "badsig", "secret").is_err());
    }

    #[test]
    fn extract_product_id_from_settings() {
        let key = test_key();
        let encrypted =
            crate::payment::crypto::aes256gcm_encrypt(r#"{"api_key":"test_key"}"#, &key).unwrap();

        let channel = PaymentChannel {
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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

    #[tokio::test]
    async fn verify_callback_parses_checkout_completed() {
        let key = test_key();
        let webhook_secret = "wh_secret_123";

        let body = serde_json::json!({
            "eventType": "checkout.completed",
            "object": {
                "id": "ch_4l0N34kxo16AhRKUHFUuXr",
                "object": "checkout",
                "order": {
                    "id": "ord_4aDwWXjMLpes4Kj4XqNnUA",
                    "status": "paid",
                    "amount": 9999
                },
                "metadata": {
                    "payment_order_id": "12345"
                }
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(&body_bytes);
        let sig = hex::encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("creem-signature", sig.parse().unwrap());

        let encrypted_secret =
            crate::payment::crypto::aes256gcm_encrypt(webhook_secret, &key).unwrap();

        let channel = PaymentChannel {
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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

        let provider = CreemProvider::new(key);
        let result = provider
            .verify_callback(&channel, &headers, &body_bytes)
            .await
            .unwrap();

        assert_eq!(result.provider_order_id, "ch_4l0N34kxo16AhRKUHFUuXr");
        assert!(matches!(result.status, PaymentStatus::Paid));
        assert_eq!(result.amount, 9999);
        assert!(result.provider_tx_id.is_none());
    }

    #[tokio::test]
    async fn verify_callback_parses_checkout_completed_subscription() {
        let key = test_key();
        let webhook_secret = "wh_secret_sub";

        let body = serde_json::json!({
            "eventType": "checkout.completed",
            "object": {
                "id": "ch_sub123",
                "object": "checkout",
                "order": {
                    "id": "ord_sub123",
                    "status": "paid",
                    "amount": 1000
                },
                "subscription": {
                    "id": "sub_abc",
                    "last_transaction_id": "tran_5yMaWzAl3jxuGJMCOrYWwk"
                },
                "metadata": {
                    "payment_order_id": "67890"
                }
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(&body_bytes);
        let sig = hex::encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("creem-signature", sig.parse().unwrap());

        let encrypted_secret =
            crate::payment::crypto::aes256gcm_encrypt(webhook_secret, &key).unwrap();

        let channel = PaymentChannel {
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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

        let provider = CreemProvider::new(key);
        let result = provider
            .verify_callback(&channel, &headers, &body_bytes)
            .await
            .unwrap();

        assert_eq!(result.provider_order_id, "ch_sub123");
        assert!(matches!(result.status, PaymentStatus::Paid));
        assert_eq!(result.amount, 1000);
        assert_eq!(
            result.provider_tx_id.as_deref(),
            Some("tran_5yMaWzAl3jxuGJMCOrYWwk")
        );
    }

    #[tokio::test]
    async fn verify_callback_parses_refund_created() {
        let key = test_key();
        let webhook_secret = "wh_secret_456";

        let body = serde_json::json!({
            "eventType": "refund.created",
            "object": {
                "id": "ref_3DB9NQFvk18TJwSqd0N6bd",
                "refund_amount": 5000,
                "checkout": {
                    "id": "ch_4l0N34kxo16AhRKUHFUuXr"
                },
                "transaction": {
                    "id": "txn_002"
                }
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(&body_bytes);
        let sig = hex::encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("creem-signature", sig.parse().unwrap());

        let encrypted_secret =
            crate::payment::crypto::aes256gcm_encrypt(webhook_secret, &key).unwrap();

        let channel = PaymentChannel {
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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

        let provider = CreemProvider::new(key);
        let result = provider
            .verify_callback(&channel, &headers, &body_bytes)
            .await
            .unwrap();

        assert_eq!(result.provider_order_id, "ch_4l0N34kxo16AhRKUHFUuXr");
        assert!(matches!(result.status, PaymentStatus::Refunded));
        assert_eq!(result.amount, 5000);
        assert_eq!(result.provider_tx_id.as_deref(), Some("txn_002"));
    }

    #[tokio::test]
    async fn cancel_is_noop() {
        let key = test_key();
        let channel = PaymentChannel {
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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

        let provider = CreemProvider::new(key);
        provider.cancel(&channel, "any_id").await.unwrap();
    }

    #[tokio::test]
    async fn refund_returns_error() {
        let key = test_key();
        let channel = PaymentChannel {
            id: test_channel_id(),
            tenant_id: None,
            provider: "creem".into(),
            name: "Creem".into(),
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

        let provider = CreemProvider::new(key);
        let result = provider.refund(&channel, "any_id", 1000, None).await;
        assert!(result.is_err());
    }
}

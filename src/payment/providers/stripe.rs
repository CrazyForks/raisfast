use std::collections::HashMap;
use std::str::FromStr;

use axum::http::HeaderMap;
use serde::Deserialize;

use crate::errors::app_error::{AppError, AppResult};
use crate::models::payment_channel::PaymentChannel;
use crate::models::payment_order::{PaymentOrder, PaymentStatus};
use crate::payment::PaymentProvider;
use crate::payment::crypto::aes256gcm_decrypt;
use crate::payment::provider::{CallbackData, ProviderResponse, ProviderStatus, RefundResponse};

#[derive(Deserialize)]
struct StripeCredentials {
    secret_key: String,
}

fn create_client(credentials: &str, encrypt_key: &[u8; 32]) -> AppResult<stripe::Client> {
    let decrypted = aes256gcm_decrypt(credentials, encrypt_key)?;
    let creds: StripeCredentials = serde_json::from_str(&decrypted).map_err(|e| {
        AppError::Internal(anyhow::Error::from(e).context("stripe credentials parse"))
    })?;
    Ok(stripe::Client::new(&creds.secret_key))
}

fn decrypt_webhook_secret(channel: &PaymentChannel, encrypt_key: &[u8; 32]) -> AppResult<String> {
    let encrypted = channel
        .webhook_secret
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("stripe webhook_secret not configured".into()))?;
    aes256gcm_decrypt(encrypted, encrypt_key)
}

fn map_stripe_status(status: &stripe::PaymentIntentStatus) -> PaymentStatus {
    match status {
        stripe::PaymentIntentStatus::Succeeded => PaymentStatus::Paid,
        stripe::PaymentIntentStatus::Canceled => PaymentStatus::Cancelled,
        stripe::PaymentIntentStatus::Processing
        | stripe::PaymentIntentStatus::RequiresAction
        | stripe::PaymentIntentStatus::RequiresCapture
        | stripe::PaymentIntentStatus::RequiresConfirmation
        | stripe::PaymentIntentStatus::RequiresPaymentMethod => PaymentStatus::Pending,
    }
}

fn stripe_error(e: stripe::StripeError) -> AppError {
    AppError::Internal(anyhow::Error::from(e).context("stripe"))
}

pub struct StripeProvider {
    encrypt_key: [u8; 32],
}

impl StripeProvider {
    pub fn new(encrypt_key: [u8; 32]) -> Self {
        Self { encrypt_key }
    }
}

#[async_trait::async_trait]
impl PaymentProvider for StripeProvider {
    fn name(&self) -> &str {
        "stripe"
    }

    async fn create(
        &self,
        channel: &PaymentChannel,
        order: &PaymentOrder,
        return_url: Option<&str>,
        _notify_url: Option<&str>,
    ) -> AppResult<ProviderResponse> {
        let client = create_client(&channel.credentials, &self.encrypt_key)?;
        let currency_str = order.currency.to_lowercase();
        let currency = stripe::Currency::from_str(&currency_str).map_err(|_| {
            AppError::BadRequest(format!("unsupported stripe currency: {currency_str}"))
        })?;

        let mut params = stripe::CreatePaymentIntent::new(order.amount, currency);
        let mut meta = HashMap::new();
        meta.insert("payment_order_id".to_string(), order.id.to_string());
        params.metadata = Some(stripe::Metadata::from(meta));

        if let Some(url) = return_url {
            params.return_url = Some(url);
        }

        let client = client.with_strategy(stripe::RequestStrategy::Idempotent(
            order.idempotency_key.clone(),
        ));
        let pi = stripe::PaymentIntent::create(&client, params)
            .await
            .map_err(stripe_error)?;

        Ok(ProviderResponse {
            provider_order_id: pi.id.to_string(),
            redirect_url: None,
            qr_code: None,
            client_secret: pi.client_secret,
        })
    }

    async fn query(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
    ) -> AppResult<ProviderStatus> {
        let client = create_client(&channel.credentials, &self.encrypt_key)?;
        let id: stripe::PaymentIntentId = provider_order_id
            .parse()
            .map_err(|e| AppError::BadRequest(format!("invalid stripe payment intent id: {e}")))?;

        let pi = stripe::PaymentIntent::retrieve(&client, &id, &[])
            .await
            .map_err(stripe_error)?;

        Ok(ProviderStatus {
            status: map_stripe_status(&pi.status),
            provider_tx_id: pi.latest_charge.as_ref().map(|c| c.id().to_string()),
            paid_at: None,
            amount: Some(pi.amount),
        })
    }

    async fn cancel(&self, channel: &PaymentChannel, provider_order_id: &str) -> AppResult<()> {
        let client = create_client(&channel.credentials, &self.encrypt_key)?;
        stripe::PaymentIntent::cancel(
            &client,
            provider_order_id,
            stripe::CancelPaymentIntent::default(),
        )
        .await
        .map_err(stripe_error)?;
        Ok(())
    }

    async fn refund(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
        amount: i64,
        reason: Option<&str>,
    ) -> AppResult<RefundResponse> {
        let client = create_client(&channel.credentials, &self.encrypt_key)?;
        let pi_id: stripe::PaymentIntentId = provider_order_id
            .parse()
            .map_err(|e| AppError::BadRequest(format!("invalid stripe payment intent id: {e}")))?;

        let mut params = stripe::CreateRefund::new();
        params.payment_intent = Some(pi_id);
        params.amount = Some(amount);
        if reason.is_some() {
            params.reason = Some(stripe::RefundReasonFilter::RequestedByCustomer);
        }

        let refund = stripe::Refund::create(&client, params)
            .await
            .map_err(stripe_error)?;

        Ok(RefundResponse {
            provider_refund_id: refund.id.to_string(),
        })
    }

    async fn verify_callback(
        &self,
        channel: &PaymentChannel,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<CallbackData> {
        let sig_header = headers
            .get("Stripe-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::BadRequest("missing Stripe-Signature".into()))?;

        let webhook_secret = decrypt_webhook_secret(channel, &self.encrypt_key)?;

        let payload = std::str::from_utf8(body)
            .map_err(|e| AppError::BadRequest(format!("invalid utf8 body: {e}")))?;

        let event = stripe::Webhook::construct_event(payload, sig_header, &webhook_secret)
            .map_err(|e| AppError::BadRequest(format!("stripe webhook verification: {e}")))?;

        let pi = match event.data.object {
            stripe::EventObject::PaymentIntent(pi) => pi,
            _ => {
                return Err(AppError::BadRequest("unexpected stripe event type".into()));
            }
        };

        Ok(CallbackData {
            provider_order_id: pi.id.to_string(),
            status: map_stripe_status(&pi.status),
            amount: pi.amount,
            provider_tx_id: pi.latest_charge.as_ref().map(|c| c.id().to_string()),
            paid_at: None,
        })
    }
}

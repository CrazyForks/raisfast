use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::payment_order::PaymentStatus;

fn validate_url(url: &str) -> Result<(), validator::ValidationError> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(validator::ValidationError::new("invalid_url"))
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreatePaymentOrderRequest {
    #[validate(length(min = 1))]
    pub order_id: String,
    pub channel_id: Option<String>,
    pub method: Option<String>,
    pub country: Option<String>,
    pub language: Option<String>,
    #[validate(custom(
        function = "validate_url",
        message = "return_url must start with http:// or https://"
    ))]
    pub return_url: Option<String>,
    pub metadata: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateRefundRequest {
    #[validate(range(min = 1))]
    pub amount: i64,
    pub reason: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreatePaymentChannelRequest {
    #[validate(length(min = 1, max = 50))]
    pub provider: String,
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    pub is_live: Option<bool>,
    #[validate(length(min = 1))]
    pub credentials: String,
    pub webhook_secret: Option<String>,
    pub settings: Option<String>,
    pub sort_order: Option<i64>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdatePaymentChannelRequest {
    pub name: Option<String>,
    pub is_live: Option<bool>,
    pub credentials: Option<String>,
    pub webhook_secret: Option<String>,
    pub settings: Option<String>,
    pub is_active: Option<bool>,
    pub sort_order: Option<i64>,
    pub version: i64,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentOrderResponse {
    pub id: String,
    pub order_id: Option<String>,
    pub title: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub amount: i64,
    pub currency: String,
    pub provider: String,
    pub provider_order_id: Option<String>,
    pub provider_method: Option<String>,
    pub status: PaymentStatus,
    pub return_url: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version: i64,
    pub provider_data: Option<String>,
    pub client_ip: Option<String>,
    pub client_language: Option<String>,
    pub client_country: Option<String>,
    pub client_user_agent: Option<String>,
    pub channel_selected_by: Option<String>,
    pub metadata: Option<String>,
    pub redirect_url: Option<String>,
    pub qr_code: Option<String>,
    pub client_secret: Option<String>,
    pub paid_at: Option<String>,
    pub cancelled_at: Option<String>,
    pub expired_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentChannelResponse {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub is_live: bool,
    pub credentials_masked: String,
    pub webhook_secret_set: bool,
    pub settings: Option<String>,
    pub is_active: bool,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sort_order: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::payment_channel::PaymentChannel> for PaymentChannelResponse {
    fn from(ch: crate::models::payment_channel::PaymentChannel) -> Self {
        Self {
            id: ch.id.to_string(),
            provider: ch.provider,
            name: ch.name,
            is_live: ch.is_live != 0,
            credentials_masked: mask_credentials(&ch.credentials),
            webhook_secret_set: ch.webhook_secret.is_some(),
            settings: ch.settings,
            is_active: ch.is_active != 0,
            sort_order: ch.sort_order,
            version: ch.version,
            created_at: ch.created_at.to_string(),
            updated_at: ch.updated_at.to_string(),
        }
    }
}

fn mask_credentials(_creds: &str) -> String {
    "[encrypted]".to_string()
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentTransactionResponse {
    pub id: String,
    pub order_id: Option<String>,
    pub tx_type: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub amount: i64,
    pub currency: String,
    pub provider_tx_id: String,
    pub status: String,
    pub created_at: String,
}

impl From<crate::models::payment_transaction::PaymentTransaction> for PaymentTransactionResponse {
    fn from(tx: crate::models::payment_transaction::PaymentTransaction) -> Self {
        Self {
            id: tx.id.to_string(),
            order_id: tx.order_id,
            tx_type: tx.tx_type,
            amount: tx.amount,
            currency: tx.currency,
            provider_tx_id: tx.provider_tx_id,
            status: tx.status,
            created_at: tx.created_at.to_string(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentRefundResponse {
    pub id: String,
    pub order_id: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub amount: i64,
    pub currency: String,
    pub reason: Option<String>,
    pub provider_refund_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::payment_refund::PaymentRefund> for PaymentRefundResponse {
    fn from(r: crate::models::payment_refund::PaymentRefund) -> Self {
        Self {
            id: r.id.to_string(),
            order_id: r.order_id,
            amount: r.amount,
            currency: r.currency,
            reason: r.reason,
            provider_refund_id: r.provider_refund_id,
            status: r.status,
            created_at: r.created_at.to_string(),
            updated_at: r.updated_at.to_string(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct AvailableChannelItem {
    pub channel_id: String,
    pub provider: String,
    pub name: String,
    pub is_recommended: bool,
    pub sort_order: i64,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct AvailableChannelsResponse {
    pub recommended_channel_id: Option<String>,
    pub channels: Vec<AvailableChannelItem>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct AvailableChannelsQuery {
    pub order_id: String,
    pub country: Option<String>,
    pub language: Option<String>,
}

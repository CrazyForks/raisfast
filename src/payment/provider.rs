use crate::errors::app_error::AppResult;
use crate::models::payment_channel::PaymentChannel;
use crate::models::payment_order::{PaymentOrder, PaymentStatus};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub provider_order_id: String,
    pub redirect_url: Option<String>,
    pub qr_code: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderStatus {
    pub status: PaymentStatus,
    pub provider_tx_id: Option<String>,
    pub paid_at: Option<String>,
    pub amount: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RefundResponse {
    pub provider_refund_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackData {
    pub provider_order_id: String,
    pub status: PaymentStatus,
    pub amount: i64,
    pub provider_tx_id: Option<String>,
    pub paid_at: Option<String>,
}

#[async_trait::async_trait]
pub trait PaymentProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn create(
        &self,
        channel: &PaymentChannel,
        order: &PaymentOrder,
        return_url: Option<&str>,
        notify_url: Option<&str>,
    ) -> AppResult<ProviderResponse>;
    async fn query(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
    ) -> AppResult<ProviderStatus>;
    async fn cancel(&self, channel: &PaymentChannel, provider_order_id: &str) -> AppResult<()>;
    async fn refund(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
        amount: i64,
        reason: Option<&str>,
    ) -> AppResult<RefundResponse>;
    async fn verify_callback(
        &self,
        channel: &PaymentChannel,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<CallbackData>;
}

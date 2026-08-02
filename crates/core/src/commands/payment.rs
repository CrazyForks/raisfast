use crate::types::snowflake_id::SnowflakeId;
pub struct CreatePaymentOrderCmd {
    pub user_id: SnowflakeId,
    pub order_id: Option<String>,
    pub title: String,
    pub amount: i64,
    pub currency: String,
    pub channel_id: SnowflakeId,
    pub provider: String,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub return_url: Option<String>,
    pub idempotency_key: String,
    pub client_ip: Option<String>,
    pub client_language: Option<String>,
    pub client_country: Option<String>,
    pub client_user_agent: Option<String>,
    pub channel_selected_by: Option<String>,
    pub metadata: Option<String>,
}

pub struct CreatePaymentChannelCmd {
    pub provider: String,
    pub name: String,
    pub is_live: bool,
    pub credentials: String,
    pub webhook_secret: Option<String>,
    pub settings: Option<String>,
    pub is_active: bool,
    pub sort_order: i64,
}

pub struct CreatePaymentTransactionCmd {
    pub payment_order_id: SnowflakeId,
    pub order_id: Option<String>,
    pub user_id: SnowflakeId,
    pub tx_type: String,
    pub amount: i64,
    pub currency: String,
    pub provider_tx_id: String,
    pub status: String,
    pub raw_payload: Option<String>,
}

pub struct CreatePaymentRefundCmd {
    pub payment_order_id: SnowflakeId,
    pub order_id: Option<String>,
    pub user_id: SnowflakeId,
    pub amount: i64,
    pub currency: String,
    pub reason: Option<String>,
    pub provider_refund_id: Option<String>,
    pub status: String,
    pub payment_tx_id: Option<i64>,
    pub metadata: Option<String>,
}

pub struct UpdatePaymentChannelCmd {
    pub id: SnowflakeId,
    pub provider: String,
    pub name: String,
    pub is_live: bool,
    pub credentials: String,
    pub webhook_secret: Option<String>,
    pub settings: Option<String>,
    pub is_active: bool,
    pub sort_order: i64,
    pub version: i64,
}

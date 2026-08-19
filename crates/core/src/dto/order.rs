use crate::types::price::Price;
use crate::types::snowflake_id::SnowflakeId;
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

#[derive(Debug, Deserialize)]
pub struct AdminOrderListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub keyword: Option<String>,
    pub status: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, Validate, ToSchema)]
pub struct CreateOrderRequest {
    #[validate(length(min = 1))]
    pub items: Vec<CreateOrderItemRequest>,
    pub currency: Option<String>,
    pub buyer_name: Option<String>,
    pub buyer_phone: Option<String>,
    pub buyer_email: Option<String>,
    pub shipping_address: Option<String>,
    pub shipping_address_id: Option<SnowflakeId>,
    pub billing_address_id: Option<SnowflakeId>,
    pub remark: Option<String>,
    pub coupon_id: Option<SnowflakeId>,
    pub coupon_code: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, Validate, ToSchema)]
pub struct CreateOrderItemRequest {
    pub product_id: SnowflakeId,
    #[validate(range(min = 1))]
    pub quantity: i64,
    pub variant_id: Option<SnowflakeId>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelOrderRequest {}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct ShipOrderRequest {
    pub tracking_no: Option<String>,
    pub carrier: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderItemResponse {
    pub id: SnowflakeId,
    pub title: String,
    pub description: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub unit_price: Price,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub quantity: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub subtotal: Price,
    pub cover_url: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub attributes: Option<serde_json::Value>,
    pub created_at: String,
}

impl From<crate::models::order_item::OrderItem> for OrderItemResponse {
    fn from(i: crate::models::order_item::OrderItem) -> Self {
        Self {
            id: i.id,
            title: i.title,
            description: i.description,
            unit_price: i.unit_price,
            quantity: i.quantity,
            subtotal: i.subtotal,
            cover_url: i.cover_url,
            attributes: i.attributes,
            created_at: i.created_at.to_string(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderResponse {
    pub id: SnowflakeId,
    pub order_no: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub subtotal: Price,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub discount_amount: Price,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub shipping_amount: Price,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total_amount: Price,
    pub currency: String,
    pub status: String,
    pub buyer_name: Option<String>,
    pub buyer_phone: Option<String>,
    pub buyer_email: Option<String>,
    pub shipping_address: Option<String>,
    pub tracking_no: Option<String>,
    pub carrier: Option<String>,
    pub remark: Option<String>,
    pub admin_remark: Option<String>,
    pub delivery_data: Option<String>,
    pub paid_at: Option<String>,
    pub completed_at: Option<String>,
    pub cancelled_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub items: Vec<OrderItemResponse>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderStatsResponse {
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total_orders: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub pending_orders: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub paid_orders: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub completed_orders: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total_revenue: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub today_orders: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub today_revenue: i64,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAdminRemarkRequest {
    pub admin_remark: String,
}

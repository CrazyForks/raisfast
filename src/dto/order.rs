use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use super::validate_optional_uuid;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateProductRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub category_id: Option<String>,
    pub product_type: Option<String>,
    pub fulfillment_type: Option<String>,
    pub delivery_hook: Option<String>,
    pub weight: Option<i64>,
    pub price: i64,
    pub currency: Option<String>,
    pub attributes: Option<String>,
    pub sort_order: Option<i64>,
    pub slug: Option<String>,
    pub content: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub image_ids: Option<String>,
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub specs: Option<String>,
    pub unit: Option<String>,
    pub min_purchase: Option<i64>,
    pub max_purchase: Option<i64>,
    pub virtual_sales: Option<i64>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateProductRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub category_id: Option<String>,
    pub product_type: Option<String>,
    pub fulfillment_type: Option<String>,
    pub delivery_hook: Option<String>,
    pub weight: Option<i64>,
    pub price: Option<i64>,
    pub currency: Option<String>,
    pub status: Option<String>,
    pub attributes: Option<String>,
    pub sort_order: Option<i64>,
    pub version: i64,
    pub slug: Option<String>,
    pub content: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub image_ids: Option<String>,
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub specs: Option<String>,
    pub unit: Option<String>,
    pub min_purchase: Option<i64>,
    pub max_purchase: Option<i64>,
    pub virtual_sales: Option<i64>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
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
    pub remark: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, Validate, ToSchema)]
pub struct CreateOrderItemRequest {
    pub product_id: String,
    #[validate(range(min = 1))]
    pub quantity: i64,
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
pub struct ProductResponse {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub product_type: String,
    pub fulfillment_type: String,
    pub delivery_hook: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub weight: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub price: i64,
    pub currency: String,
    pub status: String,
    pub attributes: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sort_order: i64,
    pub slug: Option<String>,
    pub content: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub image_ids: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub specs: Option<String>,
    pub unit: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub min_purchase: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub max_purchase: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total_sales: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub virtual_sales: i64,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub published_at: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::product::Product> for ProductResponse {
    fn from(p: crate::models::product::Product) -> Self {
        Self {
            id: p.id.to_string(),
            title: p.title,
            description: p.description,
            cover_url: p.cover_url,
            product_type: p.product_type.to_string(),
            fulfillment_type: p.fulfillment_type.to_string(),
            delivery_hook: p.delivery_hook,
            weight: p.weight,
            price: p.price,
            currency: p.currency,
            status: p.status.to_string(),
            attributes: p.attributes,
            sort_order: p.sort_order,
            slug: p.slug,
            content: p.content,
            image_ids: p.image_ids,
            original_price: p.original_price,
            specs: p.specs,
            unit: p.unit,
            min_purchase: p.min_purchase,
            max_purchase: p.max_purchase,
            total_sales: p.total_sales,
            virtual_sales: p.virtual_sales,
            meta_title: p.meta_title,
            meta_description: p.meta_description,
            published_at: p.published_at.map(|t| t.to_string()),
            version: p.version,
            created_at: p.created_at.to_string(),
            updated_at: p.updated_at.to_string(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderItemResponse {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub unit_price: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub quantity: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub subtotal: i64,
    pub cover_url: Option<String>,
    pub attributes: Option<String>,
    pub created_at: String,
}

impl From<crate::models::order_item::OrderItem> for OrderItemResponse {
    fn from(i: crate::models::order_item::OrderItem) -> Self {
        Self {
            id: i.id.to_string(),
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
    pub id: String,
    pub order_no: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub subtotal: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub discount_amount: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub shipping_amount: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total_amount: i64,
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
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAdminRemarkRequest {
    pub admin_remark: String,
}

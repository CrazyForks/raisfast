use crate::types::price::Price;
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::types::snowflake_id::SnowflakeId;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct AddToCartRequest {
    pub product_id: SnowflakeId,
    #[validate(range(min = 1))]
    pub quantity: i64,
    pub variant_id: Option<SnowflakeId>,
    pub attributes: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateCartItemRequest {
    #[validate(range(min = 1))]
    pub quantity: i64,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CartItemResponse {
    pub id: SnowflakeId,
    /// Numeric product id — the canonical key for order/cart mutations.
    pub product_id: SnowflakeId,
    /// Product slug for storefront links (may be absent on legacy data).
    pub product_slug: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub quantity: i64,
    pub attributes: Option<String>,
    pub title: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub price: Price,
    pub cover_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CartResponse {
    pub items: Vec<CartItemResponse>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total: Price,
}

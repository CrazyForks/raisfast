use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct AddToCartRequest {
    pub product_id: String,
    #[validate(range(min = 1))]
    pub quantity: i64,
    pub variant_id: Option<String>,
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
    pub id: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub quantity: i64,
    pub attributes: Option<String>,
    pub title: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub price: i64,
    pub cover_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CartResponse {
    pub items: Vec<CartItemResponse>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total: i64,
}

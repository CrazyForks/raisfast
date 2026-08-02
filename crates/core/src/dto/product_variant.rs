use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use super::validate_optional_id;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateProductVariantRequest {
    #[validate(custom(function = "validate_optional_id"))]
    pub product_id: String,
    pub sku: Option<String>,
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    pub price: i64,
    pub original_price: Option<i64>,
    pub stock: Option<i64>,
    pub attributes: Option<String>,
    pub image_url: Option<String>,
    pub weight: Option<i64>,
    pub sort_order: Option<i64>,
    pub is_active: Option<bool>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateProductVariantRequest {
    pub sku: Option<String>,
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub price: Option<i64>,
    pub original_price: Option<i64>,
    pub stock: Option<i64>,
    pub attributes: Option<String>,
    pub image_url: Option<String>,
    pub weight: Option<i64>,
    pub sort_order: Option<i64>,
    pub is_active: Option<bool>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct ProductVariantResponse {
    pub id: String,
    pub sku: Option<String>,
    pub title: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub price: i64,
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub stock: i64,
    pub attributes: Option<String>,
    pub image_url: Option<String>,
    pub weight: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sort_order: i64,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::product_variant::ProductVariant> for ProductVariantResponse {
    fn from(v: crate::models::product_variant::ProductVariant) -> Self {
        Self {
            id: v.id.to_string(),
            sku: v.sku,
            title: v.title,
            price: v.price,
            original_price: v.original_price,
            stock: v.stock,
            attributes: v.attributes,
            image_url: v.image_url,
            weight: v.weight,
            sort_order: v.sort_order,
            is_active: v.is_active,
            created_at: v.created_at.to_string(),
            updated_at: v.updated_at.to_string(),
        }
    }
}

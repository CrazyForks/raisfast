use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use super::validate_optional_uuid;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateProductVariantRequest {
    #[validate(custom(function = "validate_optional_uuid"))]
    pub product_id: String,
    pub sku: Option<String>,
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    pub price: i64,
    pub original_price: Option<i64>,
    pub stock: Option<i64>,
    pub attributes: Option<String>,
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
    pub sort_order: Option<i64>,
    pub is_active: Option<bool>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct ProductVariantResponse {
    pub id: String,
    pub product_id: String,
    pub sku: Option<String>,
    pub title: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub price: i64,
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub stock: i64,
    pub attributes: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sort_order: i64,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::product_variant::ProductVariant> for ProductVariantResponse {
    fn from(v: crate::models::product_variant::ProductVariant) -> Self {
        Self {
            id: v.document_id,
            product_id: v.product_id.to_string(),
            sku: v.sku,
            title: v.title,
            price: v.price,
            original_price: v.original_price,
            stock: v.stock,
            attributes: v.attributes,
            sort_order: v.sort_order,
            is_active: v.is_active,
            created_at: v.created_at.to_string(),
            updated_at: v.updated_at.to_string(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateUserAddressRequest {
    pub label: Option<String>,
    #[validate(length(min = 1, max = 200))]
    pub recipient_name: String,
    #[validate(length(min = 1, max = 50))]
    pub phone: String,
    #[validate(length(min = 1, max = 10))]
    pub country: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub province: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub city: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub district: Option<String>,
    #[validate(length(min = 1))]
    pub address_line1: String,
    pub address_line2: Option<String>,
    pub postal_code: Option<String>,
    pub is_default: Option<bool>,
    pub address_type: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateUserAddressRequest {
    pub label: Option<String>,
    #[validate(length(min = 1, max = 200))]
    pub recipient_name: Option<String>,
    #[validate(length(min = 1, max = 50))]
    pub phone: Option<String>,
    #[validate(length(min = 1, max = 10))]
    pub country: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub province: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub city: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub district: Option<String>,
    #[validate(length(min = 1))]
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub postal_code: Option<String>,
    pub is_default: Option<bool>,
    pub address_type: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct UserAddressResponse {
    pub id: String,
    pub label: String,
    pub recipient_name: String,
    pub phone: String,
    pub country: String,
    pub province: String,
    pub city: String,
    pub district: String,
    pub address_line1: String,
    pub address_line2: Option<String>,
    pub postal_code: Option<String>,
    pub is_default: bool,
    pub address_type: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::user_address::UserAddress> for UserAddressResponse {
    fn from(a: crate::models::user_address::UserAddress) -> Self {
        Self {
            id: a.document_id,
            label: a.label,
            recipient_name: a.recipient_name,
            phone: a.phone,
            country: a.country,
            province: a.province,
            city: a.city,
            district: a.district,
            address_line1: a.address_line1,
            address_line2: a.address_line2,
            postal_code: a.postal_code,
            is_default: a.is_default,
            address_type: a.address_type,
            created_at: a.created_at.to_string(),
            updated_at: a.updated_at.to_string(),
        }
    }
}

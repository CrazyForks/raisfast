use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

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
            id: a.id.to_string(),
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

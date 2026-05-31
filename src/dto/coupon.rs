use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateCouponRequest {
    #[validate(length(min = 1, max = 64))]
    pub code: String,
    #[validate(length(min = 1, max = 255))]
    pub title: String,
    pub coupon_type: Option<String>,
    #[validate(range(min = 1))]
    pub value: i64,
    pub min_order: Option<i64>,
    pub max_uses: Option<i64>,
    pub max_uses_per_user: Option<i64>,
    pub starts_at: Option<String>,
    pub expires_at: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateCouponRequest {
    pub title: Option<String>,
    pub value: Option<i64>,
    pub min_order: Option<i64>,
    pub max_uses: Option<i64>,
    pub max_uses_per_user: Option<i64>,
    pub starts_at: Option<String>,
    pub expires_at: Option<String>,
    pub status: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CouponResponse {
    pub id: String,
    pub code: String,
    pub title: String,
    pub coupon_type: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub value: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub min_order: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub max_uses: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub used_count: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub max_uses_per_user: i64,
    pub starts_at: Option<String>,
    pub expires_at: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::coupon::Coupon> for CouponResponse {
    fn from(c: crate::models::coupon::Coupon) -> Self {
        Self {
            id: c.id.to_string(),
            code: c.code,
            title: c.title,
            coupon_type: c.coupon_type.to_string(),
            value: c.value,
            min_order: c.min_order,
            max_uses: c.max_uses,
            used_count: c.used_count,
            max_uses_per_user: c.max_uses_per_user,
            starts_at: c.starts_at.map(|t| t.to_string()),
            expires_at: c.expires_at.map(|t| t.to_string()),
            status: c.status.to_string(),
            created_at: c.created_at.to_string(),
            updated_at: c.updated_at.to_string(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyCouponRequest {
    pub coupon_id: Option<String>,
    pub coupon_code: Option<String>,
}

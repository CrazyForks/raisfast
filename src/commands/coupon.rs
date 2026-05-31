use crate::types::snowflake_id::SnowflakeId;

pub struct CreateCouponCmd {
    pub code: String,
    pub title: String,
    pub coupon_type: String,
    pub value: i64,
    pub min_order: i64,
    pub max_uses: i64,
    pub max_uses_per_user: i64,
    pub starts_at: Option<String>,
    pub expires_at: Option<String>,
}

pub struct UpdateCouponCmd {
    pub id: SnowflakeId,
    pub title: Option<String>,
    pub value: Option<i64>,
    pub min_order: Option<i64>,
    pub max_uses: Option<i64>,
    pub max_uses_per_user: Option<i64>,
    pub starts_at: Option<String>,
    pub expires_at: Option<String>,
    pub status: Option<String>,
}

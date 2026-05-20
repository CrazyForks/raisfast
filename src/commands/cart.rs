use crate::types::snowflake_id::SnowflakeId;
pub struct CreateCartItemCmd {
    pub user_id: SnowflakeId,
    pub product_id: String,
    pub quantity: i64,
    pub attributes: Option<String>,
}

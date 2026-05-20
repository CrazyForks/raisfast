use crate::types::snowflake_id::SnowflakeId;
pub struct CreateUserAddressCmd {
    pub user_id: SnowflakeId,
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
}

pub struct UpdateUserAddressCmd {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
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
}

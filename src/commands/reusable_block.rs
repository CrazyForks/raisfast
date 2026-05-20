//! Reusable block-related commands

use crate::types::snowflake_id::SnowflakeId;
pub struct CreateReusableBlockCmd {
    pub name: String,
    pub block_type: String,
    pub content: String,
    pub description: Option<String>,
    pub created_by: Option<i64>,
}

pub struct UpdateReusableBlockCmd {
    pub id: SnowflakeId,
    pub name: Option<String>,
    pub block_type: Option<String>,
    pub content: Option<String>,
    pub description: Option<String>,
    pub updated_by: Option<i64>,
}

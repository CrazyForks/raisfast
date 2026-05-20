//! RBAC-related commands

use crate::types::snowflake_id::SnowflakeId;
pub struct CreatePermissionCmd {
    pub role_id: SnowflakeId,
    pub action: String,
    pub subject: String,
    pub fields: Option<String>,
    pub conditions: Option<String>,
}

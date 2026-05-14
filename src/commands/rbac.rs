//! RBAC-related commands

pub struct CreatePermissionCmd {
    pub role_id: i64,
    pub action: String,
    pub subject: String,
    pub fields: Option<String>,
    pub conditions: Option<String>,
}

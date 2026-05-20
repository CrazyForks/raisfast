//! Category-related commands

use crate::types::snowflake_id::SnowflakeId;

/// Create a category
pub struct CreateCategoryCmd {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
}

/// Update a category
pub struct UpdateCategoryCmd {
    pub id: SnowflakeId,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<i64>,
    pub sort_order: Option<i64>,
}

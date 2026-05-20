//! Comment-related commands

use crate::types::snowflake_id::SnowflakeId;
/// Create a comment
pub struct CreateCommentCmd {
    pub post_id: SnowflakeId,
    pub created_by: Option<i64>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub parent_id: Option<i64>,
}

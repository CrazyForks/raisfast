use crate::types::snowflake_id::SnowflakeId;

pub struct CreateProductCommentCmd {
    pub product_id: SnowflakeId,
    pub order_id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub rating: i64,
    pub title: Option<String>,
    pub content: String,
    pub images: Option<String>,
}

pub struct UpdateProductCommentCmd {
    pub id: SnowflakeId,
    pub rating: Option<i64>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub images: Option<String>,
}

pub struct AdminReplyCmd {
    pub id: SnowflakeId,
    pub admin_reply: String,
}

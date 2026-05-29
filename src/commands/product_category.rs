use crate::types::snowflake_id::SnowflakeId;

pub struct CreateProductCategoryCmd {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
}

pub struct UpdateProductCategoryCmd {
    pub id: SnowflakeId,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<i64>,
    pub sort_order: Option<i64>,
}

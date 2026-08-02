use crate::types::snowflake_id::SnowflakeId;
pub struct CreateProductVariantCmd {
    pub product_id: SnowflakeId,
    pub sku: Option<String>,
    pub title: String,
    pub price: i64,
    pub original_price: Option<i64>,
    pub stock: i64,
    pub attributes: Option<String>,
    pub image_url: Option<String>,
    pub weight: Option<i64>,
    pub sort_order: i64,
    pub is_active: bool,
}

pub struct UpdateProductVariantCmd {
    pub id: SnowflakeId,
    pub sku: Option<String>,
    pub title: String,
    pub price: i64,
    pub original_price: Option<i64>,
    pub stock: i64,
    pub attributes: Option<String>,
    pub image_url: Option<String>,
    pub weight: Option<i64>,
    pub sort_order: i64,
    pub is_active: bool,
}

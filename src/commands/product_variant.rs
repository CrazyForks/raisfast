pub struct CreateProductVariantCmd {
    pub product_id: i64,
    pub sku: Option<String>,
    pub title: String,
    pub price: i64,
    pub original_price: Option<i64>,
    pub stock: i64,
    pub attributes: Option<String>,
    pub sort_order: i64,
    pub is_active: bool,
}

pub struct UpdateProductVariantCmd {
    pub id: i64,
    pub sku: Option<String>,
    pub title: String,
    pub price: i64,
    pub original_price: Option<i64>,
    pub stock: i64,
    pub attributes: Option<String>,
    pub sort_order: i64,
    pub is_active: bool,
}

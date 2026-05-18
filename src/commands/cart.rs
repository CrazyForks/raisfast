pub struct CreateCartItemCmd {
    pub user_id: i64,
    pub product_id: String,
    pub quantity: i64,
    pub attributes: Option<String>,
}

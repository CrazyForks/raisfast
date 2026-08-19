use crate::types::price::Price;
use crate::types::snowflake_id::SnowflakeId;
pub struct CreateOrderCmd {
    pub user_id: SnowflakeId,
    pub order_no: String,
    pub subtotal: Price,
    pub discount_amount: Price,
    pub shipping_amount: Price,
    pub total_amount: Price,
    pub currency: String,
    pub buyer_name: Option<String>,
    pub buyer_phone: Option<String>,
    pub buyer_email: Option<String>,
    pub shipping_address: Option<String>,
    pub remark: Option<String>,
    pub tax_amount: Price,
    pub coupon_id: Option<i64>,
    pub shipping_address_id: Option<i64>,
    pub billing_address_id: Option<i64>,
}

pub struct CreateOrderItemCmd {
    pub order_id: SnowflakeId,
    pub product_id: Option<i64>,
    pub variant_id: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub sku: Option<String>,
    pub unit_price: Price,
    pub quantity: i64,
    pub subtotal: Price,
    pub tax_amount: Price,
    pub cover_url: Option<String>,
    pub attributes: Option<String>,
}

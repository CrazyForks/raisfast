use crate::types::price::Price;
use crate::types::snowflake_id::SnowflakeId;

pub struct CreateShippingTemplateCmd {
    pub name: String,
    pub template_type: String,
    pub first_unit: i64,
    pub first_price: Price,
    pub additional_unit: i64,
    pub additional_price: Price,
    pub free_shipping_amount: Option<Price>,
    pub regions: String,
}

pub struct UpdateShippingTemplateCmd {
    pub id: SnowflakeId,
    pub name: Option<String>,
    pub template_type: Option<String>,
    pub first_unit: Option<i64>,
    pub first_price: Option<Price>,
    pub additional_unit: Option<i64>,
    pub additional_price: Option<Price>,
    pub free_shipping_amount: Option<Price>,
    pub regions: Option<String>,
    pub status: Option<String>,
}

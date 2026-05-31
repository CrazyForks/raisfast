use crate::types::snowflake_id::SnowflakeId;

pub struct CreateShippingTemplateCmd {
    pub name: String,
    pub template_type: String,
    pub first_unit: i64,
    pub first_price: i64,
    pub additional_unit: i64,
    pub additional_price: i64,
    pub free_shipping_amount: i64,
    pub regions: String,
}

pub struct UpdateShippingTemplateCmd {
    pub id: SnowflakeId,
    pub name: Option<String>,
    pub template_type: Option<String>,
    pub first_unit: Option<i64>,
    pub first_price: Option<i64>,
    pub additional_unit: Option<i64>,
    pub additional_price: Option<i64>,
    pub free_shipping_amount: Option<i64>,
    pub regions: Option<String>,
    pub status: Option<String>,
}

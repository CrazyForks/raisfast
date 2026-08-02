use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateShippingTemplateRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    #[serde(rename = "type")]
    pub template_type: Option<String>,
    pub first_unit: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub first_price: Option<i64>,
    pub additional_unit: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub additional_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub free_shipping_amount: Option<i64>,
    pub regions: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateShippingTemplateRequest {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub template_type: Option<String>,
    pub first_unit: Option<i64>,
    pub first_price: Option<i64>,
    pub additional_unit: Option<i64>,
    pub additional_price: Option<i64>,
    pub free_shipping_amount: Option<i64>,
    pub regions: Option<String>,
    pub status: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct ShippingTemplateResponse {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub template_type: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub first_unit: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub first_price: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub additional_unit: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub additional_price: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub free_shipping_amount: i64,
    pub regions: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::shipping_template::ShippingTemplate> for ShippingTemplateResponse {
    fn from(t: crate::models::shipping_template::ShippingTemplate) -> Self {
        Self {
            id: t.id.to_string(),
            name: t.name,
            template_type: t.template_type.to_string(),
            first_unit: t.first_unit,
            first_price: t.first_price,
            additional_unit: t.additional_unit,
            additional_price: t.additional_price,
            free_shipping_amount: t.free_shipping_amount,
            regions: t.regions,
            status: t.status.to_string(),
            created_at: t.created_at.to_string(),
            updated_at: t.updated_at.to_string(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct CalculateShippingRequest {
    pub items: Vec<ShippingItem>,
    pub region: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct ShippingItem {
    pub product_id: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub quantity: i64,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CalculateShippingResponse {
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub shipping_amount: i64,
    pub details: Vec<TemplateShippingDetail>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateShippingDetail {
    pub template_id: String,
    pub template_name: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub shipping_amount: i64,
}

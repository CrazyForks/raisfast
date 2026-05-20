use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::currencies::Currency;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CurrencyResponse {
    pub id: String,
    pub code: String,
    pub name: String,
    pub decimals: i64,
    pub is_active: bool,
}

impl From<Currency> for CurrencyResponse {
    fn from(c: Currency) -> Self {
        Self {
            id: c.id.to_string(),
            code: c.code,
            name: c.name,
            decimals: c.decimals,
            is_active: c.is_active,
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCurrencyRequest {
    #[validate(
        length(min = 1),
        custom(function = "crate::dto::validate_currency_code")
    )]
    pub code: String,
    #[validate(length(min = 1))]
    pub name: String,
    pub decimals: Option<i64>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateCurrencyRequest {
    pub name: Option<String>,
    pub is_active: Option<bool>,
}

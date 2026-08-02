use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateDeviceCodeRequest {
    #[validate(length(min = 1))]
    pub access_token: String,
    #[validate(length(min = 1))]
    pub refresh_token: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ExchangeDeviceCodeRequest {
    #[validate(length(min = 1))]
    pub code: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceCodeResponse {
    pub code: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct ExchangeDeviceCodeResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: crate::dto::UserResponse,
}

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateOptionsRequest {
    pub options: HashMap<String, Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateOptionRequest {
    pub value: Value,
}

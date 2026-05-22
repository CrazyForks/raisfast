use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::reusable_block::ReusableBlock;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct ReusableBlockResponse {
    pub id: String,
    pub name: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

impl ReusableBlockResponse {
    pub fn from_block(b: ReusableBlock) -> Self {
        Self {
            id: b.id.to_string(),
            name: b.name,
            content: b.content,
            created_at: b.created_at.to_rfc3339(),
            updated_at: b.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateReusableRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    #[validate(length(min = 1))]
    pub block_type: String,
    #[validate(length(min = 1))]
    pub content: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateReusableRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: Option<String>,
    #[validate(length(min = 1))]
    pub block_type: Option<String>,
    #[validate(length(min = 1))]
    pub content: Option<String>,
    pub description: Option<String>,
}

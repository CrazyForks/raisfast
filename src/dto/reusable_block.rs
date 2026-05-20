use serde::Serialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;

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

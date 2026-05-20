use serde::Serialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;

use crate::models::page::Page;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct PageResponse {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub status: String,
    pub template: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sort_order: i64,
    pub cover_image: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl PageResponse {
    pub fn from_page(p: Page) -> Self {
        Self {
            id: p.id.to_string(),
            title: p.title,
            slug: p.slug,
            content: p.content,
            status: p.status.to_string(),
            template: p.template,
            sort_order: p.sort_order,
            cover_image: p.cover_image,
            created_at: p.created_at.to_rfc3339(),
            updated_at: p.updated_at.to_rfc3339(),
        }
    }
}

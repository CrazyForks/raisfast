//! Page-related commands

use serde::{Deserialize, Serialize};

use crate::models::page::PageStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePageCmd {
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: String,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
    pub status: PageStatus,
    pub created_by: i64,
    pub updated_by: Option<i64>,
    pub cover_image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePageCmd {
    #[serde(serialize_with = "crate::utils::id::serialize_id_as_string")]
    pub id: i64,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: Option<String>,
    pub parent_id: Option<Option<i64>>,
    pub sort_order: Option<i64>,
    pub status: Option<PageStatus>,
    pub cover_image: Option<String>,
    pub updated_by: Option<i64>,
}

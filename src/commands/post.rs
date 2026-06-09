//! Post-related commands

use crate::models::post::PostStatus;
use crate::types::snowflake_id::SnowflakeId;

/// Create a post
pub struct CreatePostCmd {
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub image_ids: Option<String>,
    pub status: PostStatus,
    pub created_by: i64,
    pub updated_by: Option<i64>,
    pub category_id: Option<i64>,
    pub tag_ids: Option<Vec<i64>>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
}

/// Update a post
pub struct UpdatePostCmd {
    pub id: SnowflakeId,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub image_ids: Option<String>,
    pub status: Option<PostStatus>,
    pub category_id: Option<i64>,
    pub tag_ids: Option<Vec<i64>>,
    pub updated_by: Option<i64>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
}

/// Query published posts
pub struct FindPublishedQuery {
    pub page: i64,
    pub page_size: i64,
    pub category_id: Option<i64>,
    pub tag_id: Option<i64>,
    pub q: Option<String>,
}

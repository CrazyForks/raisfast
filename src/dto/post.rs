use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use super::{validate_optional_uuid, validate_post_status, validate_uuid_vec};
use crate::utils::tz::Timestamp;

/// 创建文章请求体
#[derive(Debug, Clone, Deserialize, Serialize, Validate, ToSchema)]
pub struct CreatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    #[validate(length(min = 1))]
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    #[validate(custom(function = "validate_post_status"))]
    pub status: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub category_id: Option<String>,
    #[validate(custom(function = "validate_uuid_vec"))]
    pub tag_ids: Option<Vec<String>>,
}

/// 更新文章请求体
#[derive(Debug, Deserialize, Serialize, Validate, Clone, ToSchema)]
pub struct UpdatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub content: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    #[validate(custom(function = "validate_post_status"))]
    pub status: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub category_id: Option<String>,
    #[validate(custom(function = "validate_uuid_vec"))]
    pub tag_ids: Option<Vec<String>>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone, ToSchema)]
#[non_exhaustive]
pub struct PostResponse {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: String,
    pub created_by: i64,
    pub author_name: Option<String>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub tags: Vec<crate::models::post::TagBrief>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub password: Option<String>,
    pub comment_status: String,
    pub format: String,
    pub template: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
    pub reading_time: i64,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
    #[schema(value_type = Option<String>)]
    pub published_at: Option<Timestamp>,
    pub title_highlight: Option<String>,
    pub excerpt_highlight: Option<String>,
}

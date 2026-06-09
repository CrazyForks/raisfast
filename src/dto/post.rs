use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::errors::app_error::AppResult;
use crate::models::post::{CommentOpenStatus, PostStatus};
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Deserialize, Serialize, Validate, ToSchema)]
pub struct CreatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    #[validate(length(min = 1))]
    pub content: String,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub image_ids: Option<String>,
    pub status: Option<PostStatus>,
    #[validate(custom(function = "super::validate_optional_id"))]
    pub category_id: Option<String>,
    #[validate(custom(function = "super::validate_id_vec"))]
    pub tag_ids: Option<Vec<String>>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Serialize, Validate, Clone, ToSchema)]
pub struct UpdatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub content: Option<String>,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub image_ids: Option<String>,
    pub status: Option<PostStatus>,
    #[validate(custom(function = "super::validate_optional_id"))]
    pub category_id: Option<String>,
    #[validate(custom(function = "super::validate_id_vec"))]
    pub tag_ids: Option<Vec<String>>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
#[non_exhaustive]
pub struct PostResponse {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub image_ids: Option<String>,
    pub status: PostStatus,
    pub author_name: Option<String>,
    pub category_name: Option<String>,
    pub tags: Vec<crate::models::post::TagBrief>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub password: Option<String>,
    pub comment_status: CommentOpenStatus,
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
    pub created_by: Option<String>,
    pub category_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct PostListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub category_id: Option<String>,
    pub tag_id: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct AdminPostListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<PostStatus>,
    pub keyword: Option<String>,
}

impl PostResponse {
    pub fn from_post(p: crate::models::post::Post) -> AppResult<Self> {
        let status = p.status;
        let comment_status = p.comment_status;
        Ok(Self {
            id: p.id.to_string(),
            title: p.title,
            slug: p.slug,
            content: p.content,
            status,
            cover_image: p.cover_image,
            image_ids: p.image_ids,
            author_name: None,
            tags: vec![],
            view_count: p.view_count,
            is_pinned: p.is_pinned,
            password: p.password,
            comment_status,
            format: p.format,
            template: p.template,
            meta_title: p.meta_title,
            meta_description: p.meta_description,
            og_title: p.og_title,
            og_description: p.og_description,
            og_image: p.og_image,
            canonical_url: p.canonical_url,
            reading_time: p.reading_time,
            excerpt: p.excerpt,
            created_at: p.created_at,
            updated_at: p.updated_at,
            published_at: p.published_at,
            title_highlight: None,
            excerpt_highlight: None,
            created_by: None,
            category_id: None,
            category_name: None,
        })
    }
}

//! API 数据传输对象（DTO）
//!
//! 包含所有 HTTP 请求体和响应体类型。这些类型仅在 Handler 层和 Service 层使用，
//! 与数据库模型（`models::*`）解耦。

use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::models::media::Media;
use crate::models::user::User;

// ── User ──────────────────────────────────────────────────────

/// 注册请求体
#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 2, max = 50))]
    pub username: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: String,
}

/// 登录请求体
#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1, max = 128))]
    pub password: String,
}

/// 刷新令牌请求体
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// 更新用户资料请求体
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateUserRequest {
    #[validate(length(min = 2, max = 50))]
    pub username: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub avatar: Option<String>,
}

/// 修改密码请求体
#[derive(Debug, Deserialize, Validate)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 1, max = 128))]
    pub old_password: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub new_password: String,
}

/// 管理员更新角色请求体
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateRoleRequest {
    #[validate(length(min = 1))]
    pub role: String,
}

/// 用户公开信息响应
#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct UserResponse {
    pub id: String,
    pub email: String,
    pub username: String,
    pub role: String,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            email: user.email,
            username: user.username,
            role: user.role,
            avatar: user.avatar,
            bio: user.bio,
            website: user.website,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// 登录成功响应
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user: UserResponse,
}

// ── Post ──────────────────────────────────────────────────────

/// 创建文章请求体
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
pub struct CreatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    #[validate(length(min = 1))]
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub category_id: Option<String>,
    pub tag_ids: Option<Vec<String>>,
}

/// 更新文章请求体
#[derive(Debug, Deserialize, Serialize, Validate, Clone)]
pub struct UpdatePostRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub content: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub category_id: Option<String>,
    pub tag_ids: Option<Vec<String>>,
}

/// 文章 API 响应
#[derive(Debug, Serialize, Clone)]
#[non_exhaustive]
pub struct PostResponse {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub html_content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: String,
    pub author_id: String,
    pub author_name: Option<String>,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub tags: Vec<crate::models::post::TagBrief>,
    pub view_count: i64,
    pub is_pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub published_at: Option<String>,
    /// 搜索时标题高亮 HTML（含 `<em>` 标签），非搜索时为 None
    pub title_highlight: Option<String>,
    /// 搜索时内容摘要高亮 HTML，非搜索时为 None
    pub excerpt_highlight: Option<String>,
}

// ── Category ──────────────────────────────────────────────────

/// 创建分类请求体
#[derive(Debug, Deserialize, Validate)]
pub struct CreateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

/// 更新分类请求体
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

// ── Tag ───────────────────────────────────────────────────────

/// 创建标签请求体
#[derive(Debug, Deserialize, Validate)]
pub struct CreateTagRequest {
    #[validate(length(min = 1, max = 50))]
    pub name: String,
}

// ── Comment ───────────────────────────────────────────────────

/// 创建评论请求体
#[derive(Debug, Deserialize, Validate)]
pub struct CreateCommentRequest {
    #[validate(length(min = 1, max = 5000))]
    pub content: String,
    pub parent_id: Option<String>,
    #[validate(length(min = 1, max = 50))]
    pub nickname: Option<String>,
    #[validate(email)]
    pub email: Option<String>,
}

/// 更新评论状态请求体
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateCommentStatusRequest {
    #[validate(length(min = 1))]
    pub status: String,
}

// ── Media ─────────────────────────────────────────────────────

/// 媒体文件 API 响应
#[derive(Debug, Serialize)]
pub struct MediaResponse {
    pub id: String,
    pub user_id: String,
    pub filename: String,
    pub url: String,
    pub mimetype: String,
    pub size: i64,
    pub created_at: String,
}

/// 将 Media 数据库模型转换为 API 响应
pub fn media_to_response(media: &Media, base_url: &str) -> MediaResponse {
    MediaResponse {
        id: media.id.clone(),
        user_id: media.user_id.clone(),
        filename: media.filename.clone(),
        url: format!("{}/uploads/{}", base_url, media.filepath),
        mimetype: media.mimetype.clone(),
        size: media.size,
        created_at: media.created_at.clone(),
    }
}

// ── 验证辅助 ──────────────────────────────────────────────────

fn validate_password(pwd: &str) -> Result<(), validator::ValidationError> {
    let has_letter = pwd.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = pwd.chars().any(|c| c.is_ascii_digit());
    if has_letter && has_digit {
        Ok(())
    } else {
        let mut err = validator::ValidationError::new("password_strength");
        err.message = Some("password must contain both letters and digits".into());
        Err(err)
    }
}

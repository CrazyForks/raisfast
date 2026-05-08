use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

use super::{validate_comment_status, validate_optional_uuid};

/// 创建评论请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCommentRequest {
    #[validate(length(min = 1, max = 5000))]
    pub content: String,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub parent_id: Option<String>,
    #[validate(length(min = 1, max = 50))]
    pub nickname: Option<String>,
    #[validate(email)]
    pub email: Option<String>,
}

/// 更新评论状态请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateCommentStatusRequest {
    #[validate(custom(function = "validate_comment_status"))]
    pub status: String,
}

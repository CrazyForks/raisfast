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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_comment_valid() {
        let req = CreateCommentRequest {
            content: "Nice post!".to_string(),
            parent_id: None,
            nickname: Some("Guest".to_string()),
            email: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_comment_empty_content_fails() {
        let req = CreateCommentRequest {
            content: "".to_string(),
            parent_id: None,
            nickname: None,
            email: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn update_status_valid() {
        let req = UpdateCommentStatusRequest {
            status: "approved".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_status_invalid() {
        let req = UpdateCommentStatusRequest {
            status: "deleted".to_string(),
        };
        assert!(req.validate().is_err());
    }
}

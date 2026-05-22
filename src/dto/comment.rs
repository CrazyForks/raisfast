use serde::Deserialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::comment::CommentStatus;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCommentRequest {
    #[validate(length(min = 1, max = 5000))]
    pub content: String,
    #[validate(custom(function = "super::validate_optional_id"))]
    pub parent_id: Option<String>,
    #[validate(length(min = 1, max = 50))]
    pub nickname: Option<String>,
    #[validate(email)]
    pub email: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateCommentStatusRequest {
    pub status: CommentStatus,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct AdminCommentListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<CommentStatus>,
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
            status: CommentStatus::Approved,
        };
        assert!(req.validate().is_ok());
    }
}

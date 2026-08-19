use crate::types::snowflake_id::SnowflakeId;
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::product_comment::ProductCommentStatus;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateProductCommentRequest {
    pub product_id: SnowflakeId,
    pub order_id: SnowflakeId,
    #[validate(range(min = 1, max = 5))]
    pub rating: i64,
    #[validate(length(max = 200))]
    pub title: Option<String>,
    #[validate(length(min = 1, max = 5000))]
    pub content: String,
    pub images: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateProductCommentRequest {
    #[validate(range(min = 1, max = 5))]
    pub rating: Option<i64>,
    #[validate(length(max = 200))]
    pub title: Option<String>,
    #[validate(length(min = 1, max = 5000))]
    pub content: Option<String>,
    pub images: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminReplyRequest {
    pub admin_reply: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateProductCommentStatusRequest {
    pub status: ProductCommentStatus,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AdminProductCommentListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct ProductCommentResponse {
    pub id: SnowflakeId,
    pub product_id: SnowflakeId,
    pub order_id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub rating: i64,
    pub title: Option<String>,
    pub content: String,
    pub images: Option<String>,
    pub status: String,
    pub admin_reply: Option<String>,
    pub admin_replied_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::models::product_comment::ProductComment> for ProductCommentResponse {
    fn from(c: crate::models::product_comment::ProductComment) -> Self {
        Self {
            id: c.id,
            product_id: c.product_id,
            order_id: c.order_id,
            user_id: c.user_id,
            rating: c.rating,
            title: c.title,
            content: c.content,
            images: c.images,
            status: c.status.to_string(),
            admin_reply: c.admin_reply,
            admin_replied_at: c.admin_replied_at.map(|t| t.to_string()),
            created_at: c.created_at.to_string(),
            updated_at: c.updated_at.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_comment_valid() {
        let req = CreateProductCommentRequest {
            product_id: SnowflakeId::new(123),
            order_id: SnowflakeId::new(456),
            rating: 5,
            title: Some("Great".into()),
            content: "Really good".into(),
            images: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_comment_rating_too_low() {
        let req = CreateProductCommentRequest {
            product_id: SnowflakeId::new(123),
            order_id: SnowflakeId::new(456),
            rating: 0,
            title: None,
            content: "ok".into(),
            images: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_comment_rating_too_high() {
        let req = CreateProductCommentRequest {
            product_id: SnowflakeId::new(123),
            order_id: SnowflakeId::new(456),
            rating: 6,
            title: None,
            content: "ok".into(),
            images: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_comment_empty_content() {
        let req = CreateProductCommentRequest {
            product_id: SnowflakeId::new(123),
            order_id: SnowflakeId::new(456),
            rating: 3,
            title: None,
            content: "".into(),
            images: None,
        };
        assert!(req.validate().is_err());
    }
}

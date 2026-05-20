use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct TagResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl TagResponse {
    pub fn from_tag(tag: crate::models::tag::Tag) -> Self {
        Self {
            id: tag.id.to_string(),
            name: tag.name,
            slug: tag.slug,
            description: tag.description,
            created_at: tag.created_at.to_rfc3339(),
            updated_at: tag.updated_at.to_rfc3339(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateTagRequest {
    #[validate(length(min = 1, max = 50))]
    pub name: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateTagRequest {
    #[validate(length(min = 1, max = 50))]
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_tag_valid() {
        let req = CreateTagRequest {
            name: "rust".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_tag_empty_name_fails() {
        let req = CreateTagRequest {
            name: "".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn update_tag_valid() {
        let req = UpdateTagRequest {
            name: "updated".to_string(),
        };
        assert!(req.validate().is_ok());
    }
}

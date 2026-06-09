use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use super::validate_optional_id;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sort_order: i64,
    pub cover_image: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl CategoryResponse {
    pub fn from_category(cat: crate::models::category::Category) -> Self {
        Self {
            id: cat.id.to_string(),
            name: cat.name,
            slug: cat.slug,
            description: cat.description,
            parent_id: cat.parent_id.map(|v| v.to_string()),
            sort_order: cat.sort_order,
            cover_image: cat.cover_image,
            meta_title: cat.meta_title,
            meta_description: cat.meta_description,
            og_title: cat.og_title,
            og_description: cat.og_description,
            og_image: cat.og_image,
            created_at: cat.created_at.to_rfc3339(),
            updated_at: cat.updated_at.to_rfc3339(),
        }
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    #[validate(custom(function = "validate_optional_id"))]
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
    pub cover_image: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    #[validate(custom(function = "validate_optional_id"))]
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
    pub cover_image: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_category_valid() {
        let req = CreateCategoryRequest {
            name: "Tech".to_string(),
            slug: None,
            description: None,
            parent_id: None,
            sort_order: None,
            cover_image: None,
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_category_empty_name_fails() {
        let req = CreateCategoryRequest {
            name: "".to_string(),
            slug: None,
            description: None,
            parent_id: None,
            sort_order: None,
            cover_image: None,
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn update_category_valid() {
        let req = UpdateCategoryRequest {
            name: Some("New".to_string()),
            slug: None,
            description: None,
            parent_id: None,
            sort_order: None,
            cover_image: None,
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
        };
        assert!(req.validate().is_ok());
    }
}

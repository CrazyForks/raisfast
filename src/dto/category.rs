use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use super::validate_optional_uuid;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    pub description: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: Option<String>,
    pub description: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_category_valid() {
        let req = CreateCategoryRequest {
            name: "Tech".to_string(),
            description: None,
            parent_id: None,
            sort_order: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_category_empty_name_fails() {
        let req = CreateCategoryRequest {
            name: "".to_string(),
            description: None,
            parent_id: None,
            sort_order: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn update_category_valid() {
        let req = UpdateCategoryRequest {
            name: Some("New".to_string()),
            description: None,
            parent_id: None,
            sort_order: None,
        };
        assert!(req.validate().is_ok());
    }
}

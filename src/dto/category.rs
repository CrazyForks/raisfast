use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

use super::validate_optional_uuid;

/// 创建分类请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    pub description: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

/// 更新分类请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: Option<String>,
    pub description: Option<String>,
    #[validate(custom(function = "validate_optional_uuid"))]
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

/// 创建标签请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateTagRequest {
    #[validate(length(min = 1, max = 50))]
    pub name: String,
}

/// 更新标签请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateTagRequest {
    #[validate(length(min = 1, max = 50))]
    pub name: String,
}

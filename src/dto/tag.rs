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

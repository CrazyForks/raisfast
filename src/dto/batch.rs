use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::user::UserRole;

use super::validate_id_vec;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BatchRequest {
    #[validate(length(min = 1))]
    pub action: String,
    #[validate(length(min = 1), custom(function = "validate_id_vec"))]
    pub ids: Vec<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BatchRequestWithRole {
    #[validate(length(min = 1))]
    pub action: String,
    #[validate(length(min = 1), custom(function = "validate_id_vec"))]
    pub ids: Vec<String>,
    pub role: Option<UserRole>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchResponse {
    pub action: String,
    pub affected: usize,
}

impl BatchResponse {
    pub fn new(action: &str, affected: usize) -> Self {
        Self {
            action: action.to_string(),
            affected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_request_valid() {
        let req = BatchRequest {
            action: "delete".to_string(),
            ids: vec!["1234567890".to_string()],
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn batch_request_empty_action_fails() {
        let req = BatchRequest {
            action: "".to_string(),
            ids: vec!["1234567890".to_string()],
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn batch_request_empty_ids_fails() {
        let req = BatchRequest {
            action: "delete".to_string(),
            ids: vec![],
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn batch_request_with_role_valid() {
        let req = BatchRequestWithRole {
            action: "change_role".to_string(),
            ids: vec!["1234567890".to_string()],
            role: Some(UserRole::Author),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn batch_response_new() {
        let resp = BatchResponse::new("delete", 3);
        assert_eq!(resp.action, "delete");
        assert_eq!(resp.affected, 3);
    }
}

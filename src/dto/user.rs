use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::errors::app_error::AppResult;
use crate::models::user::{RegisteredVia, User, UserRole, UserStatus};
use crate::models::user_credential::AuthType;
use crate::utils::tz::Timestamp;

use super::validate_password;

pub type SocialLinks = HashMap<String, String>;
pub type UserMetadata = serde_json::Value;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 2, max = 50))]
    pub username: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct AdminCreateUserRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 2, max = 50))]
    pub username: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: String,
    pub role: Option<UserRole>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1, max = 128))]
    pub password: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct RefreshRequest {
    #[validate(length(min = 1))]
    pub refresh_token: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateUserRequest {
    #[validate(length(min = 2, max = 50))]
    pub username: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub avatar: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "Record<string, string>"))]
    pub social_links: Option<SocialLinks>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub metadata: Option<UserMetadata>,
    pub role: Option<UserRole>,
    pub status: Option<UserStatus>,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 1, max = 128))]
    pub old_password: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub new_password: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRoleRequest {
    pub role: UserRole,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ForgotPasswordRequest {
    #[validate(email)]
    pub email: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ResetPasswordRequest {
    #[validate(length(min = 1))]
    pub token: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub new_password: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct SetPasswordRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub new_password: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct SendSmsCodeRequest {
    #[validate(length(min = 5, max = 20))]
    pub phone: String,
    #[validate(length(min = 1, max = 30))]
    pub purpose: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct VerifySmsRequest {
    #[validate(length(min = 5, max = 20))]
    pub phone: String,
    #[validate(length(min = 4, max = 8))]
    pub code: String,
    #[validate(length(min = 1, max = 30))]
    pub purpose: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BindPhoneRequest {
    #[validate(length(min = 5, max = 20))]
    pub phone: String,
    #[validate(length(min = 4, max = 8))]
    pub code: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BindEmailRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: String,
}

/// Credential information response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialResponse {
    pub id: String,
    pub auth_type: AuthType,
    pub identifier: String,
    pub verified: bool,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl CredentialResponse {
    pub fn from_credential(c: crate::models::user_credential::UserCredential) -> AppResult<Self> {
        Ok(Self {
            id: c.id.to_string(),
            auth_type: c.auth_type,
            identifier: c.identifier,
            verified: c.verified == 1,
            created_at: c.created_at,
            updated_at: c.updated_at,
        })
    }
}

/// Authentication configuration response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthConfigResponse {
    pub registration_email_enabled: bool,
    pub registration_sms_enabled: bool,
    pub oauth_providers: Vec<String>,
    pub require_email_verification: bool,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct VerifyEmailRequest {
    #[validate(length(min = 1))]
    pub token: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ResendVerificationRequest {
    #[validate(email)]
    pub email: String,
}

/// User public profile response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
#[non_exhaustive]
pub struct UserResponse {
    pub id: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub username: String,
    pub role: UserRole,
    pub status: UserStatus,
    pub registered_via: RegisteredVia,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub display_name: Option<String>,
    pub slug: Option<String>,
    pub locale: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "Record<string, string>"))]
    pub social_links: Option<SocialLinks>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub metadata: Option<UserMetadata>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl UserResponse {
    pub fn from_user(user: User) -> AppResult<Self> {
        Self::build(user, None, None)
    }

    pub async fn from_user_with_contacts(pool: &crate::db::Pool, user: User) -> AppResult<Self> {
        let creds = crate::models::user_credential::find_by_user_id(pool, user.id).await?;
        let email = creds
            .iter()
            .find(|c| c.auth_type == crate::models::user_credential::AuthType::Email)
            .map(|c| c.identifier.clone());
        let phone = creds
            .iter()
            .find(|c| c.auth_type == crate::models::user_credential::AuthType::Phone)
            .map(|c| c.identifier.clone());
        Self::build(user, email, phone)
    }

    fn build(user: User, email: Option<String>, phone: Option<String>) -> AppResult<Self> {
        let role = user.role;
        let status = user.status;
        let registered_via = user.registered_via;
        Ok(Self {
            id: user.id.to_string(),
            email,
            phone,
            username: user.username,
            role,
            status,
            registered_via,
            avatar: user.avatar,
            bio: user.bio,
            website: user.website,
            display_name: user.display_name,
            slug: user.slug,
            locale: user.locale,
            social_links: crate::models::user::parse_social_links(&user.social_links),
            metadata: crate::models::user::parse_metadata(&user.metadata),
            created_at: user.created_at,
            updated_at: user.updated_at,
        })
    }
}

/// Login success response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
#[non_exhaustive]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user: UserResponse,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_valid() {
        let req = RegisterRequest {
            email: "test@example.com".to_string(),
            username: "testuser".to_string(),
            password: "Password1".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn register_request_bad_email() {
        let req = RegisterRequest {
            email: "not-an-email".to_string(),
            username: "testuser".to_string(),
            password: "Password1".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn register_request_short_username() {
        let req = RegisterRequest {
            email: "test@example.com".to_string(),
            username: "a".to_string(),
            password: "Password1".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn register_request_short_password() {
        let req = RegisterRequest {
            email: "test@example.com".to_string(),
            username: "testuser".to_string(),
            password: "short".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn register_request_password_no_digit() {
        let req = RegisterRequest {
            email: "test@example.com".to_string(),
            username: "testuser".to_string(),
            password: "passwordonly".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn login_request_valid() {
        let req = LoginRequest {
            email: "test@example.com".to_string(),
            password: "anypassword".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn login_request_empty_password() {
        let req = LoginRequest {
            email: "test@example.com".to_string(),
            password: "".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn update_user_request_valid() {
        let req = UpdateUserRequest {
            username: Some("newname".to_string()),
            bio: None,
            website: None,
            avatar: None,
            social_links: None,
            metadata: None,
            role: None,
            status: None,
            password: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_password_request_valid() {
        let req = UpdatePasswordRequest {
            old_password: "OldPass1".to_string(),
            new_password: "NewPass2".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_password_request_weak_new_password() {
        let req = UpdatePasswordRequest {
            old_password: "OldPass1".to_string(),
            new_password: "abcdefgh".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn user_response_from_user_serializes() {
        let resp = UserResponse {
            id: "doc-123".to_string(),
            email: Some("test@example.com".to_string()),
            phone: None,
            username: "test".to_string(),
            role: UserRole::Reader,
            status: UserStatus::Active,
            registered_via: RegisteredVia::Email,
            avatar: None,
            bio: None,
            website: None,
            display_name: None,
            slug: None,
            locale: None,
            social_links: None,
            metadata: None,
            created_at: "2025-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2025-01-01T00:00:00Z".parse().unwrap(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"id\":\"doc-123\""));
    }
}

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::user::User;
use crate::utils::tz::Timestamp;

use super::validate_password;

/// 注册请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 2, max = 50))]
    pub username: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: String,
}

/// 登录请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1, max = 128))]
    pub password: String,
}

/// 刷新令牌请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct RefreshRequest {
    #[validate(length(min = 1))]
    pub refresh_token: String,
}

/// 更新用户资料请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateUserRequest {
    #[validate(length(min = 2, max = 50))]
    pub username: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub avatar: Option<String>,
}

/// 修改密码请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 1, max = 128))]
    pub old_password: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub new_password: String,
}

/// 管理员更新角色请求体
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateRoleRequest {
    #[validate(length(min = 1))]
    pub role: String,
}

/// 请求密码重置
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ForgotPasswordRequest {
    #[validate(email)]
    pub email: String,
}

/// 重置密码
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ResetPasswordRequest {
    #[validate(length(min = 1))]
    pub token: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub new_password: String,
}

/// OAuth 用户设置密码
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct SetPasswordRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub new_password: String,
}

/// 发送短信验证码
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct SendSmsCodeRequest {
    #[validate(length(min = 5, max = 20))]
    pub phone: String,
    #[validate(length(min = 1, max = 30))]
    pub purpose: String,
}

/// 验证短信验证码（注册/登录）
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct VerifySmsRequest {
    #[validate(length(min = 5, max = 20))]
    pub phone: String,
    #[validate(length(min = 4, max = 8))]
    pub code: String,
    #[validate(length(min = 1, max = 30))]
    pub purpose: String,
}

/// 绑定手机号
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BindPhoneRequest {
    #[validate(length(min = 5, max = 20))]
    pub phone: String,
    #[validate(length(min = 4, max = 8))]
    pub code: String,
}

/// 绑定邮箱密码
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BindEmailRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: String,
}

/// 凭证信息响应
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialResponse {
    pub id: i64,
    pub auth_type: String,
    pub identifier: String,
    pub verified: bool,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl From<crate::models::user_credential::UserCredential> for CredentialResponse {
    fn from(c: crate::models::user_credential::UserCredential) -> Self {
        Self {
            id: c.id,
            auth_type: c.auth_type,
            identifier: c.identifier,
            verified: c.verified == 1,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

/// 认证配置响应
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthConfigResponse {
    pub registration_email_enabled: bool,
    pub registration_sms_enabled: bool,
    pub oauth_providers: Vec<String>,
    pub require_email_verification: bool,
}

/// 验证邮箱
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct VerifyEmailRequest {
    #[validate(length(min = 1))]
    pub token: String,
}

/// 重新发送验证邮件
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ResendVerificationRequest {
    #[validate(email)]
    pub email: String,
}

/// 用户公开信息响应
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
#[non_exhaustive]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub role: String,
    pub status: String,
    pub registered_via: String,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub display_name: Option<String>,
    pub slug: Option<String>,
    pub locale: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.document_id,
            username: user.username,
            role: user.role,
            status: user.status,
            registered_via: user.registered_via,
            avatar: user.avatar,
            bio: user.bio,
            website: user.website,
            display_name: user.display_name,
            slug: user.slug,
            locale: user.locale,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// 登录成功响应
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
            username: "test".to_string(),
            role: "reader".to_string(),
            status: "active".to_string(),
            registered_via: "email".to_string(),
            avatar: None,
            bio: None,
            website: None,
            display_name: None,
            slug: None,
            locale: None,
            created_at: "2025-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2025-01-01T00:00:00Z".parse().unwrap(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"id\":\"doc-123\""));
    }
}

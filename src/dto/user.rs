use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::user::User;

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
    pub email: String,
    pub username: String,
    pub role: String,
    pub phone: Option<String>,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.document_id,
            email: user.email,
            username: user.username,
            role: user.role,
            phone: user.phone,
            avatar: user.avatar,
            bio: user.bio,
            website: user.website,
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

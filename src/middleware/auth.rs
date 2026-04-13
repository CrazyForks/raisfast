//! JWT 认证提取器
//!
//! 本模块提供基于 JWT（HS256）Bearer Token 的身份验证中间件。
//! 通过实现 Axum 的 [`FromRequestParts`] trait，将三个提取器作为 handler 参数使用，
//! 框架会自动从请求头中解析并验证令牌，提取用户身份与角色信息。
//!
//! # 提取器
//!
//! | 提取器 | 允许的角色 | 用途 |
//! |---|---|---|
//! | [`AuthUser`] | 任意已认证用户 | 通用认证守卫 |
//! | [`AdminUser`] | `admin` | 管理后台操作 |
//! | [`AuthorUser`] | `admin`、`author` | 文章内容管理 |

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::AppError;
use crate::services::auth::Claims;

/// 从请求头中提取并验证 JWT claims。
///
/// 公共逻辑：读取 `Authorization: Bearer <token>` 头，验证签名和有效期。
/// 三个提取器共用此函数，避免重复代码。
fn extract_claims(parts: &mut Parts, state: &AppState) -> Result<Claims, AppError> {
    let secret = &state.config.jwt_secret;
    let auth_header = parts
        .headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    crate::services::auth::verify_token(token, secret)
}

/// 已认证用户提取器
///
/// 从请求的 `Authorization` 头中提取 Bearer Token，验证 JWT 签名后
/// 解析出用户 ID（`sub` 声明）和角色（`role` 声明）。
/// 适用于任何需要登录身份的路由，不限制角色。
///
/// # 示例
///
/// ```ignore
/// async fn get_profile(user: AuthUser) -> Json<Profile> { ... }
/// ```
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub role: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_claims(parts, state);
        async move {
            let claims = result?;
            Ok(AuthUser {
                user_id: claims.sub,
                role: claims.role,
            })
        }
    }
}

/// 管理员用户提取器
///
/// 与 [`AuthUser`] 类似，但额外校验用户角色必须为 `"admin"`。
/// 用于仅限管理员访问的敏感操作路由。
///
/// # 示例
///
/// ```ignore
/// async fn delete_user(admin: AdminUser) -> StatusCode { ... }
/// ```
#[derive(Debug, Clone)]
pub struct AdminUser {
    pub user_id: String,
    pub role: String,
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_claims(parts, state);
        async move {
            let claims = result?;
            if claims.role != "admin" {
                return Err(AppError::Forbidden);
            }
            Ok(AdminUser {
                user_id: claims.sub,
                role: claims.role,
            })
        }
    }
}

/// 作者用户提取器
///
/// 与 [`AuthUser`] 类似，但要求用户角色为 `"admin"` 或 `"author"`。
/// 用于文章创建、编辑等内容管理路由。
///
/// # 示例
///
/// ```ignore
/// async fn create_post(author: AuthorUser) -> Json<Post> { ... }
/// ```
#[derive(Debug, Clone)]
pub struct AuthorUser {
    pub user_id: String,
    pub role: String,
}

impl FromRequestParts<AppState> for AuthorUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_claims(parts, state);
        async move {
            let claims = result?;
            if claims.role != "admin" && claims.role != "author" {
                return Err(AppError::Forbidden);
            }
            Ok(AuthorUser {
                user_id: claims.sub,
                role: claims.role,
            })
        }
    }
}

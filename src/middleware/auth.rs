//! JWT / API Token 认证提取器
//!
//! 本模块提供基于 JWT（HS256）Bearer Token 和 API Token（`rblog_` 前缀）的身份验证中间件。
//! 通过实现 Axum 的 [`FromRequestParts`] trait，将提取器作为 handler 参数使用，
//! 框架会自动从请求头中解析并验证令牌，提取用户身份与角色信息。
//!
//! # 核心类型
//!
//! - [`AuthIdentity`] — 通用身份信息（`user_id` + `role` + `tenant_id`）
//!
//! # 提取器
//!
//! | 提取器 | 允许的角色 | 用途 |
//! |---|---|---|
//! | [`AuthUser`] | 任意已认证用户 | 通用认证守卫 |
//! | [`AdminUser`] | `admin` | 管理后台操作 |
//! | [`AuthorUser`] | `admin`、`author` | 文章内容管理 |
//! | [`OptionalAuth`] | 可选认证 | 条件认证场景 |

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::AppError;
use crate::utils::auth::extract_identity;

/// 通用已认证用户身份
///
/// 从 JWT claims 或 API Token 中提取的 `user_id`、`role`、`tenant_id`。
/// 各角色提取器共用此结构体，通过 `Deref` 便捷访问字段。
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub user_id: String,
    pub role: String,
    pub tenant_id: String,
}

impl std::ops::Deref for AuthUser {
    type Target = AuthIdentity;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::Deref for AdminUser {
    type Target = AuthIdentity;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::Deref for AuthorUser {
    type Target = AuthIdentity;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 已认证用户提取器
///
/// 从请求的 `Authorization` 头中提取 Bearer Token，支持 JWT 和 API Token。
/// 适用于任何需要登录身份的路由，不限制角色。
#[derive(Debug, Clone)]
pub struct AuthUser(pub AuthIdentity);

/// 可选认证提取器——无 token 时返回 None（不报错）
///
/// 用于需要根据 CT 配置动态决定是否需要认证的场景。
#[derive(Debug, Clone)]
pub struct OptionalAuth(pub Option<AuthIdentity>);

impl FromRequestParts<AppState> for OptionalAuth {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_identity(parts, state);
        async move { Ok(OptionalAuth(result.await.ok())) }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_identity(parts, state);
        async move {
            let identity = result.await?;
            Ok(AuthUser(identity))
        }
    }
}

/// 管理员用户提取器
///
/// 与 [`AuthUser`] 类似，但额外校验用户角色必须为 `"admin"`。
/// 用于仅限管理员访问的敏感操作路由。
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthIdentity);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_identity(parts, state);
        async move {
            let identity = result.await?;
            if identity.role != "admin" {
                return Err(AppError::Forbidden);
            }
            Ok(AdminUser(identity))
        }
    }
}

/// 作者用户提取器
///
/// 与 [`AuthUser`] 类似，但要求用户角色为 `"admin"` 或 `"author"`。
/// 用于文章创建、编辑等内容管理路由。
#[derive(Debug, Clone)]
pub struct AuthorUser(pub AuthIdentity);

impl FromRequestParts<AppState> for AuthorUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = extract_identity(parts, state);
        async move {
            let identity = result.await?;
            if identity.role != "admin" && identity.role != "author" {
                return Err(AppError::Forbidden);
            }
            Ok(AuthorUser(identity))
        }
    }
}

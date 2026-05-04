//! OpenAPI 规范与 Swagger UI 配置
//!
//! 使用 `utoipa` 从 handler 注解自动生成 OpenAPI 3.0 规范，
//! 通过 `/api/docs/openapi.json` 提供 JSON spec，
//! `/api/docs` 重定向到在线 Swagger UI。

use axum::http::StatusCode;
#[cfg(feature = "openapi")]
use axum::response::Redirect;
use axum::response::{IntoResponse, Response};
use utoipa::OpenApi;

use crate::handlers::dto;

/// OpenAPI 规范定义
///
/// 从 handler 的 `#[utoipa::path]` 注解和 DTO 的 `ToSchema` 自动收集。
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::handlers::health::health,
        crate::handlers::health::liveness,
        crate::handlers::health::readiness,
        crate::handlers::auth::register,
        crate::handlers::auth::login,
        crate::handlers::auth::refresh,
        crate::handlers::auth::logout,
        crate::handlers::auth::forgot_password,
        crate::handlers::auth::reset_password,
        crate::handlers::auth::set_password,
        crate::handlers::api_token::create,
        crate::handlers::api_token::list,
        crate::handlers::api_token::delete,
        crate::handlers::user::get_me,
        crate::handlers::user::update_me,
        crate::handlers::user::change_password,
        crate::handlers::user::list_users,
        crate::handlers::user::get_user,
        crate::handlers::user::update_role,
        crate::handlers::category::list,
        crate::handlers::category::create,
        crate::handlers::category::update,
        crate::handlers::category::delete,
        crate::handlers::tag::list,
        crate::handlers::tag::create,
        crate::handlers::tag::update,
        crate::handlers::tag::delete,
        crate::handlers::post::list,
        crate::handlers::post::get,
        crate::handlers::post::create,
        crate::handlers::post::update,
        crate::handlers::post::delete,
    ),
    components(
        schemas(
            dto::RegisterRequest,
            dto::LoginRequest,
            dto::RefreshRequest,
            dto::UpdateUserRequest,
            dto::UpdatePasswordRequest,
            dto::UpdateRoleRequest,
            dto::UserResponse,
            dto::LoginResponse,
            dto::CreatePostRequest,
            dto::UpdatePostRequest,
            dto::PostResponse,
            dto::CreateCategoryRequest,
            dto::UpdateCategoryRequest,
            dto::CreateTagRequest,
            dto::CreateCommentRequest,
            dto::UpdateCommentStatusRequest,
            dto::MediaResponse,
            crate::handlers::api_token::CreateTokenRequest,
            crate::models::post::TagBrief,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "health", description = "健康检查"),
        (name = "auth", description = "认证"),
        (name = "tokens", description = "API Token 管理"),
        (name = "users", description = "用户"),
        (name = "posts", description = "文章"),
        (name = "categories", description = "分类"),
        (name = "tags", description = "标签"),
        (name = "comments", description = "评论"),
        (name = "media", description = "媒体"),
    )
)]
pub struct ApiDoc;

/// JWT Bearer Auth 安全方案
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::Http::new(
                        utoipa::openapi::security::HttpAuthScheme::Bearer,
                    ),
                ),
            );
        }
    }
}

/// 提供 OpenAPI JSON 规范
pub async fn serve_openapi_json() -> Response {
    let spec = ApiDoc::openapi();
    let json = serde_json::to_string_pretty(&spec).unwrap_or_default();
    (StatusCode::OK, [("Content-Type", "application/json")], json).into_response()
}

/// 重定向到在线 Swagger UI（仅在 `openapi` feature 启用时编译）
#[cfg(feature = "openapi")]
pub async fn redirect_to_swagger() -> Redirect {
    let spec_url = "http://localhost:9898/api/docs/openapi.json";
    Redirect::temporary(&format!("https://petstore.swagger.io/?url={spec_url}"))
}

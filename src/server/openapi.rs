//! OpenAPI specification and Swagger UI configuration
//!
//! Uses `utoipa` to auto-generate OpenAPI 3.0 specs from handler annotations,
//! served via `/api/docs/openapi.json` as JSON spec,
//! and `/api/docs` redirects to the online Swagger UI.

use axum::http::StatusCode;
#[cfg(feature = "openapi")]
use axum::response::Redirect;
use axum::response::{IntoResponse, Response};
use utoipa::OpenApi;

use crate::dto;

/// OpenAPI specification definition
///
/// Auto-collected from handler `#[utoipa::path]` annotations and DTO `ToSchema` implementations.
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
        (name = "health", description = "Health Check"),
        (name = "auth", description = "Authentication"),
        (name = "tokens", description = "API Token Management"),
        (name = "users", description = "Users"),
        (name = "posts", description = "Posts"),
        (name = "categories", description = "Categories"),
        (name = "tags", description = "Tags"),
        (name = "comments", description = "Comments"),
        (name = "media", description = "Media"),
    )
)]
pub struct ApiDoc;

/// JWT Bearer Auth security scheme
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

/// Serve the OpenAPI JSON specification
pub async fn serve_openapi_json() -> Response {
    let spec = ApiDoc::openapi();
    let json = serde_json::to_string_pretty(&spec).unwrap_or_default();
    (StatusCode::OK, [("Content-Type", "application/json")], json).into_response()
}

/// Redirect to the online Swagger UI (only compiled when `openapi` feature is enabled)
#[cfg(feature = "openapi")]
pub async fn redirect_to_swagger() -> Redirect {
    let spec_url = "http://localhost:9898/api/docs/openapi.json";
    Redirect::temporary(&format!("https://petstore.swagger.io/?url={spec_url}"))
}

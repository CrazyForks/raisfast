//! OpenAPI specification and Swagger UI configuration
//!
//! Uses `utoipa` to auto-generate OpenAPI 3.0 specs from handler annotations,
//! served via `/api/docs/openapi.json` as JSON spec,
//! and `/api/docs` redirects to the online Swagger UI.

use axum::http::StatusCode;
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
        crate::handlers::comment::list,
        crate::handlers::comment::list_all,
        crate::handlers::comment::create,
        crate::handlers::comment::create_guest,
        crate::handlers::comment::delete,
        crate::handlers::comment::update_status,
        crate::handlers::comment::admin_list,
        crate::handlers::comment::admin_update_status,
        crate::handlers::comment::admin_delete,
        crate::handlers::comment::admin_batch,
        crate::handlers::media::upload,
        crate::handlers::media::list,
        crate::handlers::media::delete,
        crate::handlers::media::stats,
        crate::handlers::media::admin_upload,
        crate::handlers::media::admin_list,
        crate::handlers::media::admin_delete,
        crate::handlers::media::admin_batch,
        crate::handlers::page::list,
        crate::handlers::page::get_by_slug,
        crate::handlers::page::sitemap,
        crate::handlers::page::admin_list,
        crate::handlers::page::admin_get,
        crate::handlers::page::create,
        crate::handlers::page::update,
        crate::handlers::page::delete,
        crate::handlers::page::update_status,
        crate::handlers::page::reorder,
        crate::handlers::page::admin_batch,
        crate::handlers::reusable_block::list_reusable,
        crate::handlers::reusable_block::get_reusable,
        crate::handlers::reusable_block::create_reusable,
        crate::handlers::reusable_block::update_reusable,
        crate::handlers::reusable_block::delete_reusable,
        crate::handlers::reusable_block::admin_batch,
        crate::handlers::rbac::list_roles,
        crate::handlers::rbac::create_role,
        crate::handlers::rbac::update_role,
        crate::handlers::rbac::delete_role,
        crate::handlers::rbac::get_permissions,
        crate::handlers::rbac::set_permissions,
        crate::handlers::rbac::admin_batch,
        crate::handlers::tenant::list_tenants,
        crate::handlers::tenant::get_tenant,
        crate::handlers::tenant::create_tenant,
        crate::handlers::tenant::update_tenant,
        crate::handlers::tenant::delete_tenant,
        crate::handlers::tenant::admin_batch,
        crate::handlers::options::get_public_options,
        crate::handlers::options::list_options,
        crate::handlers::options::get_option,
        crate::handlers::options::update_options,
        crate::handlers::options::set_option,
        crate::handlers::options::delete_option,
        crate::handlers::product::list_active,
        crate::handlers::product::get_product,
        crate::handlers::product::admin_list,
        crate::handlers::product::admin_create,
        crate::handlers::product::admin_update,
        crate::handlers::product::admin_delete,
        crate::handlers::order::create_order,
        crate::handlers::order::list_orders,
        crate::handlers::order::get_order,
        crate::handlers::order::cancel_order_handler,
        crate::handlers::order::confirm_receipt,
        crate::handlers::order::admin_list,
        crate::handlers::order::admin_get,
        crate::handlers::order::admin_ship,
        crate::handlers::order::admin_cancel,
        crate::handlers::order::admin_pay,
        crate::handlers::order::admin_refund,
        crate::handlers::order::admin_update_remark,
        crate::handlers::order::admin_stats,
        crate::handlers::payment::create_payment_order_handler,
        crate::handlers::payment::list_user_orders,
        crate::handlers::payment::get_payment_order_handler,
        crate::handlers::payment::cancel_payment_order_handler,
        crate::handlers::payment::list_order_transactions,
        crate::handlers::payment::list_order_refunds,
        crate::handlers::payment::handle_callback,
        crate::handlers::payment::list_available_channels_handler,
        crate::handlers::payment::admin_list_channels,
        crate::handlers::payment::admin_create_channel,
        crate::handlers::payment::admin_get_channel,
        crate::handlers::payment::admin_update_channel,
        crate::handlers::payment::admin_delete_channel,
        crate::handlers::payment::admin_list_orders,
        crate::handlers::payment::admin_get_order,
        crate::handlers::payment::admin_refund_order,
        crate::handlers::payment::admin_list_transactions,
        crate::handlers::payment::admin_list_refunds,
        crate::handlers::wallet::list_wallets,
        crate::handlers::wallet::get_wallet,
        crate::handlers::wallet::list_transactions,
        crate::handlers::wallet::list_all_transactions,
        crate::handlers::wallet::list_all_wallets,
        crate::handlers::wallet::list_all_transactions_admin,
        crate::handlers::wallet::admin_credit,
        crate::handlers::wallet::admin_debit,
        crate::handlers::wallet::list_user_transactions,
        crate::handlers::wallet::list_user_all_transactions,
        crate::handlers::wallet::admin_reversal,
        crate::handlers::stats::overview,
        crate::handlers::stats::content_stats,
        crate::handlers::stats::trends,
        crate::handlers::oauth::redirect_to_provider,
        crate::handlers::oauth::callback,
        crate::handlers::oauth::list_providers,
        crate::handlers::oauth::list_bindings,
        crate::handlers::oauth::unbind,
        crate::handlers::cron::list,
        crate::handlers::cron::get,
        crate::handlers::cron::create,
        crate::handlers::cron::update,
        crate::handlers::cron::toggle,
        crate::handlers::cron::delete,
        crate::handlers::cron::logs,
        crate::handlers::cron::cleanup_logs,
        crate::handlers::cron::admin_batch,
        crate::handlers::currencies::list_currencies,
        crate::handlers::currencies::get_currency,
        crate::handlers::currencies::create_currency,
        crate::handlers::currencies::update_currency,
        crate::handlers::rss::feed,
        crate::handlers::content_revision::list_revisions,
        crate::handlers::content_revision::get_revision,
        crate::handlers::content_revision::restore_revision,
        crate::handlers::content_revision::diff_revisions,
        crate::handlers::plugin::list,
        crate::handlers::plugin::get,
        crate::handlers::plugin::enable,
        crate::handlers::plugin::disable,
        crate::handlers::plugin::reload,
        crate::handlers::plugin::remove,
        crate::handlers::plugin::admin_batch,
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
            dto::CreateTokenRequest,
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
        (name = "pages", description = "Pages"),
        (name = "reusable_blocks", description = "Reusable Blocks"),
        (name = "rbac", description = "RBAC"),
        (name = "tenants", description = "Tenants"),
        (name = "options", description = "Options"),
        (name = "products", description = "Products"),
        (name = "orders", description = "Orders"),
        (name = "payments", description = "Payments"),
        (name = "wallets", description = "Wallets"),
        (name = "stats", description = "Statistics"),
        (name = "oauth", description = "OAuth"),
        (name = "cron", description = "Cron Schedules"),
        (name = "currencies", description = "Currencies"),
        (name = "rss", description = "RSS"),
        (name = "revisions", description = "Content Revisions"),
        (name = "plugins", description = "Plugins"),
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

        let server = utoipa::openapi::ServerBuilder::new()
            .url("/")
            .description(Some("Current host"))
            .build();
        openapi.servers = Some(vec![server]);

        let prefix = crate::constants::API_PREFIX;
        let paths = std::mem::take(&mut openapi.paths.paths);
        for (path, item) in paths {
            let full_path = if path.starts_with(prefix) {
                path
            } else {
                format!("{prefix}{path}")
            };
            openapi.paths.paths.insert(full_path, item);
        }
    }
}

/// Serve the OpenAPI JSON specification
pub async fn serve_openapi_json() -> Response {
    let spec = ApiDoc::openapi();
    let json = serde_json::to_string_pretty(&spec).unwrap_or_default();
    (StatusCode::OK, [("Content-Type", "application/json")], json).into_response()
}

/// Serve Scalar API documentation UI (only compiled when `openapi` feature is enabled)
#[cfg(feature = "openapi")]
pub async fn serve_scalar_ui() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>RaisFast API Docs</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <style>
        body { margin: 0; }
        #login-bar {
            display: flex; align-items: center; gap: 8px;
            padding: 8px 16px; background: #1a1a2e; color: #eee;
            font-family: system-ui, sans-serif; font-size: 14px;
            position: sticky; top: 0; z-index: 9999;
        }
        #login-bar input {
            padding: 6px 10px; border: 1px solid #444; border-radius: 4px;
            background: #222; color: #eee; font-size: 13px;
        }
        #login-bar button {
            padding: 6px 16px; border: none; border-radius: 4px;
            background: #6366f1; color: #fff; cursor: pointer; font-size: 13px;
        }
        #login-bar button:hover { background: #4f46e5; }
        #login-bar .status { margin-left: 8px; font-size: 12px; }
        #login-bar .ok { color: #4ade80; }
        #login-bar .err { color: #f87171; }
    </style>
</head>
<body>
    <div id="login-bar">
        <span>Quick Login:</span>
        <input id="email" type="email" placeholder="Email" value="admin@raisfast.dev" style="width:180px" />
        <input id="password" type="password" placeholder="Password" value="admin123" style="width:140px" />
        <button onclick="doLogin()">Login</button>
        <span id="status" class="status"></span>
    </div>
    <div id="scalar-container"></div>
    <script>
    async function doLogin() {
        const s = document.getElementById('status');
        s.textContent = 'Logging in...'; s.className = 'status';
        try {
            const email = document.getElementById('email').value;
            const password = document.getElementById('password').value;
            const res = await fetch('/api/v1/auth/login', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ email, password })
            });
            if (!res.ok) throw new Error(await res.text());
            const data = await res.json();
            const token = data.access_token || data.data?.access_token;
            if (!token) throw new Error('No token in response');
            localStorage.setItem('raisfast_token', token);
            s.textContent = 'Token saved'; s.className = 'status ok';
            applyToken(token);
        } catch (e) {
            s.textContent = 'Failed: ' + e.message; s.className = 'status err';
        }
    }
    function applyToken(token) {
        const evt = new CustomEvent('scalar-update-auth', {
            detail: { authKey: 'bearer_auth', token: 'Bearer ' + token }
        });
        window.dispatchEvent(evt);
    }
    </script>
    <script id="api-reference" data-url="/api/docs/openapi.json"></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
</body>
</html>"#;
    ([("Content-Type", "text/html; charset=utf-8")], html).into_response()
}

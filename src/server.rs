//! HTTP 服务器：路由组装、中间件、启动与优雅关闭。

mod openapi;

use std::sync::Arc;
use std::time::Duration;

use crate::AppState;
use crate::cache::MemoryCache;
use crate::config::app::AppConfig;
use crate::handlers::{
    api_token, auth, category, comment, cron, health, media, options, page, plugin, post, rbac,
    rss, sse, stats, tag, tenant, user, workflow, ws,
};
use crate::middleware::locale::locale_middleware;
use crate::middleware::metrics;
use crate::middleware::rate_limit::{
    RateLimiterSet, comment_rate_limit, global_rate_limit, login_rate_limit, register_rate_limit,
};
use axum::Extension;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::middleware::from_fn;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post as http_post, put};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::Level;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteInfo {
    pub method: String,
    pub path: String,
    pub source: String,
    pub source_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct RouteRegistry {
    routes: Vec<RouteInfo>,
}

impl RouteRegistry {
    pub fn record(&mut self, method: &str, path: &str, source: &str, source_name: &str) {
        self.routes.push(RouteInfo {
            method: method.to_string(),
            path: path.to_string(),
            source: source.to_string(),
            source_name: source_name.to_string(),
        });
    }

    pub fn into_vec(self) -> Vec<RouteInfo> {
        self.routes
    }
}

/// 构建 CORS 中间件。
fn build_cors(config: &AppConfig) -> CorsLayer {
    match &config.cors_origins {
        Some(origins) => {
            let allow: Vec<HeaderValue> = origins
                .split(',')
                .filter_map(|o: &str| o.trim().parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(allow)
                .allow_methods(Any)
                .allow_headers(Any)
        }
        None => CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    }
}

macro_rules! reg_route {
    ($router:ident, $registry:ident, $path:literal, $handler:expr, $source:expr, $name:expr, [$($method:literal),+ $(,)?]) => {
        $router = $router.route($path, $handler);
        $($registry.record($method, concat!("/api/v1", $path), $source, $name);)+
    };
}

async fn build_app(config: &AppConfig, limiters: RateLimiterSet) -> anyhow::Result<axum::Router> {
    let upload_dir = config.upload_dir.clone();
    let static_dir = config.static_dir.clone();
    let max_upload = config.max_upload_size;

    let mut registry = RouteRegistry::default();

    let mut state = crate::build_app_state(config).await?;
    let pool = state.pool.clone();
    let eventbus = state.eventbus.clone();
    let worker_pool = pool.clone();

    if config.worker_enabled {
        let cache_for_workers: Arc<dyn crate::cache::CacheStore> = Arc::new(MemoryCache::new());
        spawn_workers(
            worker_pool,
            &eventbus,
            config,
            state.plugins.clone(),
            state.search.clone(),
            cache_for_workers,
        )
        .await;
    }

    let cors = build_cors(config);
    let mut api_v1 = axum::Router::new();

    reg_route!(
        api_v1,
        registry,
        "/auth/register",
        http_post(auth::register).layer(from_fn(register_rate_limit)),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/login",
        http_post(auth::login).layer(from_fn(login_rate_limit)),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/refresh",
        http_post(auth::refresh),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/logout",
        http_post(auth::logout),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/forgot-password",
        http_post(auth::forgot_password),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/reset-password",
        http_post(auth::reset_password),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/set-password",
        http_post(auth::set_password),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/config",
        get(auth::auth_config),
        "system",
        "auth",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/sms/send",
        http_post(auth::send_sms_code),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/sms/verify",
        http_post(auth::verify_sms),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/phone/bind",
        http_post(auth::bind_phone),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/verify-email",
        http_post(auth::verify_email),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/resend-verification",
        http_post(auth::resend_verification),
        "system",
        "auth",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/oauth/{provider}",
        get(crate::handlers::oauth::redirect_to_provider),
        "system",
        "auth",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/oauth/{provider}/callback",
        get(crate::handlers::oauth::callback),
        "system",
        "auth",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/oauth/providers",
        get(crate::handlers::oauth::list_providers),
        "system",
        "auth",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/oauth/bindings",
        get(crate::handlers::oauth::list_bindings),
        "system",
        "auth",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/auth/oauth/{provider}/unbind",
        delete(crate::handlers::oauth::unbind),
        "system",
        "auth",
        ["DELETE"]
    );

    reg_route!(
        api_v1,
        registry,
        "/tokens",
        get(api_token::list).post(api_token::create),
        "system",
        "tokens",
        ["GET", "POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/tokens/{id}",
        delete(api_token::delete),
        "system",
        "tokens",
        ["DELETE"]
    );
    reg_route!(
        api_v1,
        registry,
        "/users/me",
        get(user::get_me).put(user::update_me),
        "system",
        "users",
        ["GET", "PUT"]
    );
    reg_route!(
        api_v1,
        registry,
        "/users/me/password",
        put(user::change_password),
        "system",
        "users",
        ["PUT"]
    );
    reg_route!(
        api_v1,
        registry,
        "/users/{id}",
        get(user::get_user),
        "system",
        "users",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/users/{id}/role",
        put(user::update_role),
        "system",
        "users",
        ["PUT"]
    );
    reg_route!(
        api_v1,
        registry,
        "/users",
        get(user::list_users),
        "system",
        "users",
        ["GET"]
    );
    // ── 内置模块路由（根据 builtins 配置条件注册） ──

    if config.builtins.blog {
        reg_route!(
            api_v1,
            registry,
            "/categories",
            get(category::list).post(category::create),
            "system",
            "categories",
            ["GET", "POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/categories/{id}",
            put(category::update).delete(category::delete),
            "system",
            "categories",
            ["PUT", "DELETE"]
        );
        reg_route!(
            api_v1,
            registry,
            "/tags",
            get(tag::list).post(tag::create),
            "system",
            "tags",
            ["GET", "POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/tags/{id}",
            delete(tag::delete),
            "system",
            "tags",
            ["DELETE"]
        );
        reg_route!(
            api_v1,
            registry,
            "/posts",
            get(post::list).post(post::create),
            "system",
            "posts",
            ["GET", "POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/posts/{slug}",
            get(post::get).put(post::update).delete(post::delete),
            "system",
            "posts",
            ["GET", "PUT", "DELETE"]
        );
        reg_route!(
            api_v1,
            registry,
            "/posts/{slug}/comments",
            get(comment::list),
            "system",
            "comments",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/posts/{slug}/comments",
            http_post(comment::create_guest).layer(from_fn(comment_rate_limit)),
            "system",
            "comments",
            ["POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/posts/{slug}/comments/authed",
            http_post(comment::create),
            "system",
            "comments",
            ["POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/comments/{id}",
            delete(comment::delete),
            "system",
            "comments",
            ["DELETE"]
        );
        reg_route!(
            api_v1,
            registry,
            "/comments/{id}/status",
            put(comment::update_status),
            "system",
            "comments",
            ["PUT"]
        );
        reg_route!(
            api_v1,
            registry,
            "/comments",
            get(comment::list_all),
            "system",
            "comments",
            ["GET"]
        );
    }

    if config.builtins.pages {
        reg_route!(
            api_v1,
            registry,
            "/pages",
            get(page::list).post(page::create),
            "system",
            "pages",
            ["GET", "POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/pages/sitemap",
            get(page::sitemap),
            "system",
            "pages",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/pages/{slug}",
            get(page::get_by_slug),
            "system",
            "pages",
            ["GET"]
        );
    }

    if config.builtins.media {
        reg_route!(
            api_v1,
            registry,
            "/media/upload",
            http_post(media::upload).layer(RequestBodyLimitLayer::new(max_upload)),
            "system",
            "media",
            ["POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/media",
            get(media::list),
            "system",
            "media",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/media/stats",
            get(media::stats),
            "system",
            "media",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/media/{id}",
            delete(media::delete),
            "system",
            "media",
            ["DELETE"]
        );
    }
    reg_route!(
        api_v1,
        registry,
        "/events",
        get(sse::subscribe),
        "system",
        "sse",
        ["GET"]
    );

    if config.websocket_enabled {
        tracing::info!("WebSocket enabled at /api/v1/ws");
        reg_route!(
            api_v1,
            registry,
            "/ws",
            get(ws::ws_handler),
            "system",
            "ws",
            ["GET"]
        );
    }
    if config.graphql_enabled {
        tracing::info!("GraphQL enabled at /api/v1/graphql");
        reg_route!(
            api_v1,
            registry,
            "/graphql",
            get(crate::graphql::handler::graphiql_handler)
                .post(crate::graphql::handler::graphql_handler),
            "system",
            "graphql",
            ["GET", "POST"]
        );
    }

    if config.builtins.blog {
        reg_route!(
            api_v1,
            registry,
            "/admin/posts",
            get(post::admin_list),
            "system",
            "admin/posts",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/posts/{slug}",
            get(post::admin_get),
            "system",
            "admin/posts",
            ["GET"]
        );
    }

    if config.builtins.pages {
        reg_route!(
            api_v1,
            registry,
            "/admin/pages",
            get(page::admin_list),
            "system",
            "admin/pages",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/pages/{id}",
            get(page::admin_get).put(page::update).delete(page::delete),
            "system",
            "admin/pages",
            ["GET", "PUT", "DELETE"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/pages/{id}/status",
            put(page::update_status),
            "system",
            "admin/pages",
            ["PUT"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/pages/reorder",
            put(page::reorder),
            "system",
            "admin/pages",
            ["PUT"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/reusable-blocks",
            get(page::list_reusable).post(page::create_reusable),
            "system",
            "admin/pages",
            ["GET", "POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/reusable-blocks/{id}",
            get(page::get_reusable)
                .put(page::update_reusable)
                .delete(page::delete_reusable),
            "system",
            "admin/pages",
            ["GET", "PUT", "DELETE"]
        );
    }
    reg_route!(
        api_v1,
        registry,
        "/admin/plugins",
        get(plugin::list),
        "system",
        "admin/plugins",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/plugins/{id}",
        get(plugin::get).delete(plugin::remove),
        "system",
        "admin/plugins",
        ["GET", "DELETE"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/plugins/{id}/enable",
        http_post(plugin::enable),
        "system",
        "admin/plugins",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/plugins/{id}/disable",
        http_post(plugin::disable),
        "system",
        "admin/plugins",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/plugins/{id}/reload",
        http_post(plugin::reload),
        "system",
        "admin/plugins",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/crons",
        get(cron::list).post(cron::create),
        "system",
        "admin/crons",
        ["GET", "POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/crons/{id}",
        get(cron::get).put(cron::update).delete(cron::delete),
        "system",
        "admin/crons",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/crons/{id}/toggle",
        http_post(cron::toggle),
        "system",
        "admin/crons",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/crons/logs",
        get(cron::logs),
        "system",
        "admin/crons",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/crons/logs/cleanup",
        http_post(cron::cleanup_logs),
        "system",
        "admin/crons",
        ["POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/rbac/roles",
        get(rbac::list_roles).post(rbac::create_role),
        "system",
        "admin/rbac",
        ["GET", "POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/rbac/roles/{id}",
        put(rbac::update_role).delete(rbac::delete_role),
        "system",
        "admin/rbac",
        ["PUT", "DELETE"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/rbac/roles/{id}/permissions",
        get(rbac::get_permissions).put(rbac::set_permissions),
        "system",
        "admin/rbac",
        ["GET", "PUT"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/stats",
        get(stats::overview),
        "system",
        "admin/stats",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/stats/content/{table}",
        get(stats::content_stats),
        "system",
        "admin/stats",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/stats/trends",
        get(stats::trends),
        "system",
        "admin/stats",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/options/public",
        get(options::get_public_options),
        "system",
        "options",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/options",
        get(options::list_options).put(options::update_options),
        "system",
        "admin/options",
        ["GET", "PUT"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/options/{key}",
        get(options::get_option)
            .put(options::set_option)
            .delete(options::delete_option),
        "system",
        "admin/options",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/tenants",
        get(tenant::list_tenants).post(tenant::create_tenant),
        "system",
        "admin/tenants",
        ["GET", "POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/tenants/{id}",
        get(tenant::get_tenant)
            .put(tenant::update_tenant)
            .delete(tenant::delete_tenant),
        "system",
        "admin/tenants",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/audit",
        get(crate::audit::handler::list),
        "system",
        "admin/audit",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/audit/{id}",
        get(crate::audit::handler::get),
        "system",
        "admin/audit",
        ["GET"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/webhooks",
        get(crate::webhook::handler::list).post(crate::webhook::handler::create),
        "system",
        "admin/webhooks",
        ["GET", "POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/webhooks/{id}",
        get(crate::webhook::handler::get)
            .put(crate::webhook::handler::update)
            .delete(crate::webhook::handler::delete),
        "system",
        "admin/webhooks",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/content-types",
        get(crate::content_type::handler::list_schemas)
            .post(crate::content_type::handler::create_schema),
        "system",
        "admin/content-types",
        ["GET", "POST"]
    );
    reg_route!(
        api_v1,
        registry,
        "/admin/content-types/{singular}",
        get(crate::content_type::handler::get_schema)
            .put(crate::content_type::handler::update_schema)
            .delete(crate::content_type::handler::delete_schema),
        "system",
        "admin/content-types",
        ["GET", "PUT", "DELETE"]
    );
    if config.builtins.workflow {
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows",
            get(workflow::list).post(workflow::create),
            "system",
            "admin/workflows",
            ["GET", "POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows/{id}",
            get(workflow::get).delete(workflow::delete),
            "system",
            "admin/workflows",
            ["GET", "DELETE"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows/{id}/start",
            http_post(workflow::start),
            "system",
            "admin/workflows",
            ["POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows/instances",
            get(workflow::list_instances),
            "system",
            "admin/workflows",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows/instances/{id}",
            get(workflow::get_instance),
            "system",
            "admin/workflows",
            ["GET"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows/instances/{id}/execute",
            http_post(workflow::execute_step),
            "system",
            "admin/workflows",
            ["POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows/instances/{id}/cancel",
            http_post(workflow::cancel_instance),
            "system",
            "admin/workflows",
            ["POST"]
        );
        reg_route!(
            api_v1,
            registry,
            "/admin/workflows/instances/{id}/logs",
            get(workflow::get_step_logs),
            "system",
            "admin/workflows",
            ["GET"]
        );
    }

    api_v1 = api_v1
        .layer(from_fn(global_rate_limit))
        .layer(Extension(limiters))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));

    api_v1 =
        crate::content_type::handler::register_content_routes(api_v1, &state.content_type_registry);

    for ct in state.content_type_registry.all() {
        let plural = &ct.plural;
        let name = &ct.singular;
        if ct.is_single() {
            registry.record(
                "GET",
                &format!("/api/v1/cms/{}", name),
                "content_type",
                name,
            );
            registry.record(
                "PUT",
                &format!("/api/v1/cms/{}", name),
                "content_type",
                name,
            );
            registry.record(
                "GET",
                &format!("/api/v1/admin/cms/{}", name),
                "content_type",
                name,
            );
        } else {
            for (method, suffix) in [
                ("GET", ""),
                ("POST", ""),
                ("GET", "/{id_or_slug}"),
                ("PUT", "/{id_or_slug}"),
                ("DELETE", "/{id_or_slug}"),
            ] {
                registry.record(
                    method,
                    &format!("/api/v1/cms/{}{}", plural, suffix),
                    "content_type",
                    name,
                );
            }
            registry.record(
                "GET",
                &format!("/api/v1/admin/cms/{}", plural),
                "content_type",
                name,
            );
            registry.record(
                "GET",
                &format!("/api/v1/admin/cms/{}/{{id_or_slug}}", plural),
                "content_type",
                name,
            );
        }
    }

    api_v1 = api_v1
        .route(
            "/cms/{*path}",
            axum::routing::any(crate::content_type::handler::dynamic_cms_handler),
        )
        .route(
            "/admin/cms/{*path}",
            axum::routing::any(crate::content_type::handler::dynamic_admin_cms_handler),
        )
        .route("/routes", get(list_routes))
        .route("/health", get(health::health));

    registry.record("GET", "/api/v1/health", "system", "health");

    {
        let plugin_routes = state.plugins.all_plugin_routes().await;
        for (method, path, ext_id) in &plugin_routes {
            registry.record(method, path, "plugin", ext_id);
        }
    }

    registry.record("GET", "/api/v1/routes", "system", "system");

    let routes_vec = registry.into_vec();
    state.route_registry = Arc::new(routes_vec);

    let app = axum::Router::new()
        .route("/health", get(health::health))
        .route("/healthz", get(health::liveness))
        .route("/readyz", get(health::readiness))
        .route("/metrics", get(metrics::metrics_endpoint))
        .route("/feed.xml", get(rss::feed))
        .nest("/api/v1", api_v1)
        .nest_service("/uploads", ServeDir::new(&upload_dir))
        .nest_service("/static", ServeDir::new(&static_dir))
        .fallback(handle_plugin_route)
        .layer(from_fn(locale_middleware))
        .layer(from_fn(metrics::track_metrics))
        .layer(from_fn(crate::middleware::request_id::inject_request_id))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    let method = request.method().as_str();
                    let uri = request.uri().path();
                    let version = match request.version() {
                        axum::http::Version::HTTP_09 => "0.9",
                        axum::http::Version::HTTP_10 => "1.0",
                        axum::http::Version::HTTP_11 => "1.1",
                        axum::http::Version::HTTP_2 => "2.0",
                        axum::http::Version::HTTP_3 => "3.0",
                        _ => "unknown",
                    };
                    tracing::span!(
                        Level::INFO,
                        "request",
                        method,
                        uri,
                        version,
                        request_id = tracing::field::Empty
                    )
                })
                .on_request(|request: &axum::extract::Request, _span: &tracing::Span| {
                    tracing::info!(
                        method = %request.method(),
                        path = %request.uri().path(),
                        "--> request start"
                    );
                })
                .on_response(
                    |response: &axum::response::Response,
                     latency: Duration,
                     _span: &tracing::Span| {
                        tracing::info!(
                            status = %response.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "<-- request done"
                        );
                    },
                ),
        )
        .layer(cors)
        .with_state(state);

    let app = app.route("/api/docs/openapi.json", get(openapi::serve_openapi_json));

    #[cfg(feature = "openapi")]
    let app = app
        .route("/api/docs", get(openapi::redirect_to_swagger))
        .route("/api/docs/", get(openapi::redirect_to_swagger));

    Ok(app)
}

/// 启动 HTTP 服务器，监听请求直到收到关闭信号。
pub async fn start(config: &AppConfig) -> anyhow::Result<()> {
    metrics::init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        env = %config.env,
        "starting rust-blog server"
    );
    let tz = crate::utils::tz::parse_tz_or_utc(&config.timezone);
    tracing::info!("site timezone: {}", tz);
    crate::utils::tz::set_site_tz(tz);
    let addr = format!("{}:{}", config.host, config.port);
    let limiters = RateLimiterSet::from_config(config);

    let cleanup_limiters = limiters.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            cleanup_limiters.global.cleanup_expired().await;
            cleanup_limiters.register.cleanup_expired().await;
            cleanup_limiters.login.cleanup_expired().await;
            cleanup_limiters.comment.cleanup_expired().await;
            cleanup_limiters.api_token.cleanup_expired().await;
        }
    });

    let app = build_app(config, limiters).await?;

    match (&config.tls_cert_path, &config.tls_key_path) {
        (Some(_cert), Some(_key)) => {
            #[cfg(feature = "tls")]
            {
                use axum_server::tls_rustls::RustlsConfig;
                let tls_config = RustlsConfig::from_pem_file(_cert, _key).await?;
                tracing::info!("server listening on https://{}", addr);
                println!("server listening on https://{}", addr);
                let socket_addr: std::net::SocketAddr = addr.parse()?;
                axum_server::bind_rustls(socket_addr, tls_config)
                    .serve(app.into_make_service())
                    .await?;
            }
            #[cfg(not(feature = "tls"))]
            {
                tracing::warn!(
                    "TLS_CERT_PATH and TLS_KEY_PATH set but 'tls' feature not enabled. \
                      Falling back to HTTP."
                );
                tracing::info!(
                    "server listening on http://{} (pid={})",
                    addr,
                    std::process::id()
                );
                println!("server listening on http://{}", addr);
                let start = std::time::Instant::now();
                let listener = TcpListener::bind(&addr).await?;
                tracing::info!(
                    startup_ms = start.elapsed().as_millis() as u64,
                    "server ready to accept connections"
                );
                axum::serve(listener, app.into_make_service())
                    .with_graceful_shutdown(shutdown_signal())
                    .await?;

                tracing::info!("server shutdown complete");
            }
        }
        _ => {
            tracing::info!("server listening on http://{}", addr);
            println!("server listening on http://{}", addr);
            let listener = TcpListener::bind(&addr).await?;
            axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
    }

    Ok(())
}

/// 监听 ctrl+c (SIGINT) 和 SIGTERM 实现优雅关闭。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl+c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("received SIGINT (ctrl+c), starting graceful shutdown");
        },
        _ = terminate => {
            tracing::info!("received SIGTERM, starting graceful shutdown");
        },
    }
}

/// 插件路由 fallback。
///
/// 当 axum 路由未匹配时，尝试分发给插件的 `manifest.routes` 声明式路由。
/// 若所有插件均未处理，返回 404。
async fn handle_plugin_route(
    auth: crate::middleware::auth::AuthUser,
    State(state): State<AppState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use serde_json::json;

    let path = {
        let mut s = req.uri().path().to_string();
        if let Some(q) = req.uri().query() {
            s.push('?');
            s.push_str(q);
        }
        s
    };
    let method = req.method().to_string();

    let headers_json: serde_json::Value = {
        let mut map = serde_json::Map::new();
        for (key, value) in req.headers() {
            if let Ok(v) = value.to_str() {
                map.insert(key.to_string(), serde_json::Value::String(v.to_string()));
            }
        }
        serde_json::Value::Object(map)
    };

    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024).await;
    let body_str = body_bytes
        .ok()
        .and_then(|b| String::from_utf8(b.to_vec()).ok());

    let result = state
        .plugins
        .dispatch_route(
            &path,
            &method,
            body_str.as_deref(),
            Some(&headers_json),
            &auth,
        )
        .await;

    match result {
        Some(response) => response,
        None => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(json!({
                "code": 40400,
                "message": "not found",
                "data": null
            })),
        )
            .into_response(),
    }
}

/// 启动 EventBus 后台订阅者，将业务事件转发给插件系统。
pub fn spawn_event_subscriber(
    eventbus: crate::eventbus::EventBus,
    plugins: Arc<crate::plugins::PluginManager>,
) {
    use crate::eventbus::Event;
    use crate::plugins::HookPoint;

    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => match event.as_ref() {
                    Event::PostCreated { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins.dispatch_action(HookPoint::PostCreated, &json).await;
                    }
                    Event::PostUpdated { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins.dispatch_action(HookPoint::PostUpdated, &json).await;
                    }
                    Event::PostDeleted { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins.dispatch_action(HookPoint::PostDeleted, &json).await;
                    }
                    Event::CommentCreated { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins
                            .dispatch_action(HookPoint::CommentCreated, &json)
                            .await;
                    }
                    _ => {}
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("eventbus subscriber lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });
}

/// 启动审计日志订阅者，将所有业务事件写入 `audit_log` 表。
pub fn spawn_audit_subscriber(
    eventbus: crate::eventbus::EventBus,
    audit: Arc<crate::audit::AuditService>,
    tenant_service: Arc<crate::services::tenant::TenantService>,
) {
    use crate::eventbus::Event;

    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        let default_tenant: &str = "default";
        let _ = tenant_service;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let (action, subject, subject_id, actor_id, detail): (
                        &str,
                        &str,
                        String,
                        Option<String>,
                        Option<String>,
                    ) = match event.as_ref() {
                        Event::PostCreated {
                            id,
                            title,
                            author_id,
                            ..
                        } => (
                            "create",
                            "post",
                            id.clone(),
                            Some(author_id.clone()),
                            Some(format!("title={title}")),
                        ),
                        Event::PostUpdated { id, slug } => (
                            "update",
                            "post",
                            id.clone(),
                            None,
                            Some(format!("slug={slug}")),
                        ),
                        Event::PostDeleted { id, slug } => (
                            "delete",
                            "post",
                            id.clone(),
                            None,
                            Some(format!("slug={slug}")),
                        ),
                        Event::CommentCreated {
                            id, author_name, ..
                        } => (
                            "create",
                            "comment",
                            id.clone(),
                            None,
                            Some(format!("author={author_name}")),
                        ),
                        Event::CommentDeleted { id } => {
                            ("delete", "comment", id.clone(), None, None)
                        }
                        Event::ContentCreated {
                            content_type, id, ..
                        } => ("create", content_type.as_str(), id.clone(), None, None),
                        Event::ContentUpdated { content_type, id } => {
                            ("update", content_type.as_str(), id.clone(), None, None)
                        }
                        Event::ContentDeleted { content_type, id } => {
                            ("delete", content_type.as_str(), id.clone(), None, None)
                        }
                        Event::UserRegistered { id, username, .. } => (
                            "register",
                            "user",
                            id.clone(),
                            None,
                            Some(format!("username={username}")),
                        ),
                        Event::UserLoggedIn { id, success } => (
                            "login",
                            "user",
                            id.clone(),
                            Some(id.clone()),
                            Some(format!("success={success}")),
                        ),
                        Event::MediaUploaded {
                            id,
                            filename,
                            uploader_id,
                        } => (
                            "upload",
                            "media",
                            id.clone(),
                            Some(uploader_id.clone()),
                            Some(format!("filename={filename}")),
                        ),
                        Event::MediaDeleted { id } => ("delete", "media", id.clone(), None, None),
                        Event::PasswordResetRequested { user_id, email, .. } => (
                            "password_reset_request",
                            "user",
                            user_id.clone(),
                            None,
                            Some(format!("email={email}")),
                        ),
                        Event::EmailVerificationRequested { user_id, email, .. } => (
                            "email_verification_request",
                            "user",
                            user_id.clone(),
                            None,
                            Some(format!("email={email}")),
                        ),
                        _ => continue,
                    };

                    if let Err(e) = audit
                        .log(
                            default_tenant,
                            actor_id.as_deref(),
                            None,
                            action,
                            subject,
                            Some(&subject_id),
                            detail.as_deref(),
                            None,
                            None,
                        )
                        .await
                    {
                        tracing::warn!(%action, %subject, error = %e, "failed to write audit log");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("audit subscriber lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });
}

/// 启动 Webhook 事件投递订阅者
pub fn spawn_webhook_subscriber(
    eventbus: crate::eventbus::EventBus,
    webhook_service: Arc<crate::webhook::WebhookService>,
) {
    use crate::eventbus::Event;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|e| {
            tracing::error!("webhook http client init failed: {e}");
            panic!("webhook client failure");
        });

    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let event_type = match event.as_ref() {
                        Event::PostCreated { .. } => "post.created",
                        Event::PostUpdated { .. } => "post.updated",
                        Event::PostDeleted { .. } => "post.deleted",
                        Event::CommentCreated { .. } => "comment.created",
                        Event::CommentDeleted { .. } => "comment.deleted",
                        Event::ContentCreated { .. } => {
                            continue;
                        }
                        Event::ContentUpdated { .. } => {
                            continue;
                        }
                        Event::ContentDeleted { .. } => {
                            continue;
                        }
                        Event::UserRegistered { .. } => "user.registered",
                        Event::UserLoggedIn { .. } => "user.loggedIn",
                        Event::MediaUploaded { .. } => "media.uploaded",
                        Event::MediaDeleted { .. } => "media.deleted",
                        Event::PasswordResetRequested { .. } => "user.passwordResetRequested",
                        Event::EmailVerificationRequested { .. } => {
                            "user.emailVerificationRequested"
                        }
                        _ => continue,
                    };

                    let payload_value = serde_json::to_value(event.as_ref()).unwrap_or_default();
                    let timestamp = chrono::Utc::now().to_rfc3339();
                    let webhook_payload = crate::webhook::model::WebhookPayload {
                        event: event_type.to_string(),
                        data: payload_value,
                        timestamp,
                    };
                    let body = match serde_json::to_vec(&webhook_payload) {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!("webhook payload serialize error: {e}");
                            continue;
                        }
                    };

                    let subs = match webhook_service.find_enabled("default").await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("webhook find_enabled error: {e}");
                            continue;
                        }
                    };

                    for sub in subs {
                        let events: Vec<String> =
                            serde_json::from_str(&sub.events).unwrap_or_default();
                        if !events.iter().any(|e| {
                            e == event_type || e == "*" || event_type.starts_with(&format!("{e}."))
                        }) {
                            continue;
                        }

                        let signature = crate::webhook::service::WebhookService::sign_payload(
                            &sub.secret,
                            &body,
                        );
                        let url = sub.url.clone();
                        let body_clone = body.clone();
                        let client = client.clone();
                        tokio::spawn(async move {
                            let result = client
                                .post(&url)
                                .header("Content-Type", "application/json")
                                .header("X-Webhook-Signature", format!("sha256={signature}"))
                                .header("X-Webhook-Event", event_type)
                                .body(body_clone)
                                .send()
                                .await;
                            match result {
                                Ok(resp) => {
                                    tracing::debug!(
                                        url = %url,
                                        status = %resp.status(),
                                        "webhook delivered"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        url = %url,
                                        error = %e,
                                        "webhook delivery failed"
                                    );
                                }
                            }
                        });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("webhook subscriber lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });
}

/// 启动 Worker 子系统（CronScheduler + JobEnqueuer + WorkerRunner）
async fn spawn_workers(
    pool: crate::db::Pool,
    eventbus: &crate::eventbus::EventBus,
    config: &AppConfig,
    plugins: Arc<crate::plugins::PluginManager>,
    search: Arc<dyn crate::search::SearchEngine>,
    cache: Arc<dyn crate::cache::CacheStore>,
) {
    use crate::worker::{
        CronScheduler, JobEnqueuer, JobHandlerRegistry, PluginCronDispatcher, SqliteJobQueue,
        WorkerRunner, seed_defaults,
    };

    let queue = Arc::new(SqliteJobQueue::new(pool.clone()));

    if let Err(e) = async {
        sqlx::query(include_str!("../migrations/006_jobs.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../migrations/007_cron_schedules.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../migrations/008_cron_execution_log.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../migrations/020_password_reset.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../migrations/021_phone.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../migrations/022_email_verification.sql"))
            .execute(&pool)
            .await?;
        if config.cron_seed_enabled {
            seed_defaults(&pool, &config.cron_schedules).await?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await
    {
        tracing::warn!("worker migration/seed error: {e}");
    }

    let mut registry = JobHandlerRegistry::new();
    crate::worker::handlers::register_all(
        &mut registry,
        pool.clone(),
        Arc::new(config.clone()),
        search,
        cache,
        crate::notifier::build_email_sender(config),
        crate::notifier::build_sms_sender(config),
    );

    let cron = CronScheduler::new(
        pool,
        queue.clone(),
        Duration::from_millis(config.worker_cron_tick_ms),
    );
    cron.spawn();

    JobEnqueuer::spawn(eventbus, queue.clone());

    let runner = WorkerRunner::new(
        queue,
        Arc::new(registry),
        Duration::from_millis(config.worker_poll_interval_ms),
    )
    .with_plugin_dispatcher(Arc::new(PluginCronDispatcher::new(plugins)));
    runner.spawn(config.worker_concurrency);

    tracing::info!(
        "worker system started: concurrency={}, poll={}ms",
        config.worker_concurrency,
        config.worker_poll_interval_ms,
    );
}

async fn list_routes(State(state): State<AppState>) -> impl IntoResponse {
    use serde_json::json;

    let mut routes: Vec<RouteInfo> = state.route_registry.as_ref().clone();
    routes.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.path.cmp(&b.path)));

    axum::Json(json!({
        "code": 0,
        "data": routes,
        "message": "ok"
    }))
}

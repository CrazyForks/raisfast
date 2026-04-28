//! API 集成测试
//!
//! 覆盖所有 31 个 API 端点。使用 axum::Router + 内存 SQLite 数据库，
//! 通过 tower::ServiceExt::oneshot 发送请求，验证响应状态码和 JSON 结构。
//!
//! # 运行方式
//!
//! ```bash
//! cargo test
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware::from_fn;
use axum::routing::{delete, get, post as http_post, put};
use http_body_util::BodyExt;
use rust_blog::AppState;
use rust_blog::cache::MemoryCache;
use rust_blog::config::app::AppConfig;
use rust_blog::handlers::{
    api_token as h_token, auth as h_auth, category as h_cat, comment as h_cmt, cron as h_cron,
    health as h_health, media as h_media, options as h_options, plugin as h_plugin, post as h_post,
    rbac as h_rbac, rss as h_rss, sse as h_sse, stats as h_stats, tag as h_tag, tenant as h_tenant,
    user as h_user,
};
use rust_blog::middleware::locale::locale_middleware;
use rust_blog::middleware::rate_limit::{
    RateLimiterSet, comment_rate_limit, global_rate_limit, login_rate_limit, register_rate_limit,
};
use rust_blog::plugins::PluginManager;
use rust_blog::repositories::{
    CachedPostRepository, SqlxCategoryRepository, SqlxCommentRepository, SqlxMediaRepository,
    SqlxOptionsRepository, SqlxPostRepository, SqlxRbacRepository, SqlxRefreshTokenRepository,
    SqlxTagRepository, SqlxTenantRepository, SqlxUserRepository,
};
use rust_blog::search::NoopSearchEngine;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::limit::RequestBodyLimitLayer;

// ── helpers ──────────────────────────────────────────────────────

pub(crate) fn test_config() -> AppConfig {
    let mut cfg = AppConfig::test_defaults();
    cfg.upload_dir = std::env::temp_dir()
        .join("hello-axum-test-uploads")
        .to_string_lossy()
        .into();
    cfg.base_url = "http://localhost:9000".into();
    cfg
}

pub(crate) async fn test_pool() -> rust_blog::db::Pool {
    #[cfg(feature = "db-sqlite")]
    {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/001_init.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/002_add_indexes.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/003_plugin_storage.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/006_jobs.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/007_cron_schedules.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/008_cron_execution_log.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/009_options.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/010_rbac.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/011_tenants.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/012_audit_log.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/013_webhook_subscriptions.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/014_extensions.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/015_api_tokens.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/016_content_revisions.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/017_workflows.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/018_media_dimensions.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/019_oauth.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/020_password_reset.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/021_phone.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/022_email_verification.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }
}

pub(crate) async fn test_app() -> (axum::Router, AppState) {
    let pool = test_pool().await;
    let config = Arc::new(test_config());
    let state = AppState {
        pool: pool.clone(),
        config: config.clone(),
        jwt_decoding_key: jsonwebtoken::DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        plugins: PluginManager::new(config.clone()).await,
        eventbus: rust_blog::eventbus::EventBus::new(256),
        post_repo: Arc::new(CachedPostRepository::new(
            SqlxPostRepository::new(pool.clone()),
            Arc::new(MemoryCache::new()),
            None,
        )),
        user_repo: Arc::new(SqlxUserRepository::new(pool.clone())),
        category_repo: Arc::new(SqlxCategoryRepository::new(pool.clone())),
        tag_repo: Arc::new(SqlxTagRepository::new(pool.clone())),
        comment_repo: Arc::new(SqlxCommentRepository::new(pool.clone())),
        media_repo: Arc::new(SqlxMediaRepository::new(pool.clone())),
        refresh_token_repo: Arc::new(SqlxRefreshTokenRepository::new(pool.clone())),
        search: Arc::new(NoopSearchEngine),
        content_type_registry: Arc::new(rust_blog::content_type::ContentTypeRegistry::new()),
        options: Arc::new(
            rust_blog::services::options::OptionsService::new(Arc::new(
                SqlxOptionsRepository::new(pool.clone()),
            ))
            .await,
        ),
        rbac: Arc::new(rust_blog::services::rbac::RbacService::new(Arc::new(
            SqlxRbacRepository::new(pool.clone()),
        ))),
        tenant: Arc::new(rust_blog::services::tenant::TenantService::new(Arc::new(
            SqlxTenantRepository::new(pool.clone()),
        ))),
        audit: Arc::new(rust_blog::audit::AuditService::new(pool.clone())),
        webhook: Arc::new(rust_blog::webhook::WebhookService::new(pool.clone())),
        workflow: Arc::new(rust_blog::services::workflow::WorkflowService::new(
            pool.clone(),
        )),
        storage: rust_blog::storage::create_storage(&config).expect("failed to create storage"),
        cms_cache: Arc::new(dashmap::DashMap::new()),
        oauth_registry: Arc::new(rust_blog::oauth::OAuthProviderRegistry::default()),
        email_sender: rust_blog::notifier::build_email_sender(&config),
        sms_sender: rust_blog::notifier::build_sms_sender(&config),
        route_registry: Arc::new(Vec::new()),
    };
    let max_upload = state.config.max_upload_size;

    let api_v1 = axum::Router::new()
        .route(
            "/auth/register",
            http_post(h_auth::register).layer(from_fn(register_rate_limit)),
        )
        .route(
            "/auth/login",
            http_post(h_auth::login).layer(from_fn(login_rate_limit)),
        )
        .route("/auth/refresh", http_post(h_auth::refresh))
        .route("/auth/logout", http_post(h_auth::logout))
        .route("/tokens", get(h_token::list).post(h_token::create))
        .route("/tokens/{id}", delete(h_token::delete))
        .route("/users/me", get(h_user::get_me).put(h_user::update_me))
        .route("/users/me/password", put(h_user::change_password))
        .route("/users/{id}", get(h_user::get_user))
        .route("/users/{id}/role", put(h_user::update_role))
        .route("/users", get(h_user::list_users))
        .route("/categories", get(h_cat::list).post(h_cat::create))
        .route("/categories/{id}", put(h_cat::update).delete(h_cat::delete))
        .route("/tags", get(h_tag::list).post(h_tag::create))
        .route("/tags/{id}", delete(h_tag::delete))
        .route("/posts", get(h_post::list).post(h_post::create))
        .route(
            "/posts/{slug}",
            get(h_post::get).put(h_post::update).delete(h_post::delete),
        )
        .route(
            "/posts/{slug}/comments",
            get(h_cmt::list)
                .post(h_cmt::create_guest)
                .layer(from_fn(comment_rate_limit)),
        )
        .route("/posts/{slug}/comments/authed", http_post(h_cmt::create))
        .route("/comments/{id}", delete(h_cmt::delete))
        .route("/comments/{id}/status", put(h_cmt::update_status))
        .route(
            "/media/upload",
            http_post(h_media::upload).layer(RequestBodyLimitLayer::new(max_upload)),
        )
        .route("/media", get(h_media::list))
        .route("/media/{id}", delete(h_media::delete))
        .route("/events", get(h_sse::subscribe))
        .route("/admin/crons", get(h_cron::list).post(h_cron::create))
        .route(
            "/admin/crons/{id}",
            get(h_cron::get).put(h_cron::update).delete(h_cron::delete),
        )
        .route("/admin/crons/{id}/toggle", http_post(h_cron::toggle))
        .route("/admin/crons/logs", get(h_cron::logs))
        .route("/admin/crons/logs/cleanup", http_post(h_cron::cleanup_logs))
        .route("/admin/plugins", get(h_plugin::list))
        .route(
            "/admin/plugins/{id}",
            get(h_plugin::get).delete(h_plugin::remove),
        )
        .route("/admin/plugins/{id}/enable", http_post(h_plugin::enable))
        .route("/admin/plugins/{id}/disable", http_post(h_plugin::disable))
        .route("/admin/plugins/{id}/reload", http_post(h_plugin::reload))
        .route(
            "/admin/rbac/roles",
            get(h_rbac::list_roles).post(h_rbac::create_role),
        )
        .route(
            "/admin/rbac/roles/{id}",
            put(h_rbac::update_role).delete(h_rbac::delete_role),
        )
        .route(
            "/admin/rbac/roles/{id}/permissions",
            get(h_rbac::get_permissions).put(h_rbac::set_permissions),
        )
        .route("/admin/stats", get(h_stats::overview))
        .route("/admin/stats/content/{table}", get(h_stats::content_stats))
        .route("/admin/stats/trends", get(h_stats::trends))
        .route("/options/public", get(h_options::get_public_options))
        .route(
            "/admin/options",
            get(h_options::list_options).put(h_options::update_options),
        )
        .route(
            "/admin/options/{key}",
            get(h_options::get_option)
                .put(h_options::set_option)
                .delete(h_options::delete_option),
        )
        .route(
            "/admin/tenants",
            get(h_tenant::list_tenants).post(h_tenant::create_tenant),
        )
        .route(
            "/admin/tenants/{id}",
            get(h_tenant::get_tenant)
                .put(h_tenant::update_tenant)
                .delete(h_tenant::delete_tenant),
        )
        .route("/admin/audit", get(rust_blog::audit::handler::list))
        .route("/admin/audit/{id}", get(rust_blog::audit::handler::get))
        .route(
            "/admin/webhooks",
            get(rust_blog::webhook::handler::list).post(rust_blog::webhook::handler::create),
        )
        .route(
            "/admin/webhooks/{id}",
            get(rust_blog::webhook::handler::get)
                .put(rust_blog::webhook::handler::update)
                .delete(rust_blog::webhook::handler::delete),
        )
        .route(
            "/admin/workflows",
            get(rust_blog::handlers::workflow::list).post(rust_blog::handlers::workflow::create),
        )
        .route(
            "/admin/workflows/{id}",
            get(rust_blog::handlers::workflow::get).delete(rust_blog::handlers::workflow::delete),
        )
        .route(
            "/admin/workflows/{id}/start",
            http_post(rust_blog::handlers::workflow::start),
        )
        .route(
            "/admin/workflows/instances",
            get(rust_blog::handlers::workflow::list_instances),
        )
        .route(
            "/admin/workflows/instances/{id}",
            get(rust_blog::handlers::workflow::get_instance),
        )
        .route(
            "/admin/workflows/instances/{id}/execute",
            http_post(rust_blog::handlers::workflow::execute_step),
        )
        .route(
            "/admin/workflows/instances/{id}/cancel",
            http_post(rust_blog::handlers::workflow::cancel_instance),
        )
        .route(
            "/admin/workflows/instances/{id}/logs",
            get(rust_blog::handlers::workflow::get_step_logs),
        )
        .layer(from_fn(global_rate_limit))
        .layer(axum::Extension(RateLimiterSet::new_default()));

    let app = axum::Router::new()
        .route("/health", get(h_health::health))
        .route("/feed.xml", get(h_rss::feed))
        .nest("/api/v1", api_v1)
        .layer(from_fn(locale_middleware))
        .with_state(state.clone());

    (app, state)
}

pub(crate) async fn send(app: &mut axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

pub(crate) async fn send_raw(app: &mut axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

pub(crate) fn post_json(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

pub(crate) fn post_json_auth(path: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

pub(crate) fn put_json_auth(path: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

pub(crate) fn get_req(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn get_auth(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn delete_auth(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn make_token(user_id: &str, role: &str) -> String {
    rust_blog::services::auth::generate_access_token_for_test(user_id, role)
}

pub(crate) async fn register_and_login(
    app: &mut axum::Router,
    email: &str,
    username: &str,
    password: &str,
) -> (String, String) {
    let (status, body) = send(
        app,
        post_json(
            "/api/v1/auth/register",
            json!({"email": email, "username": username, "password": password}),
        ),
    )
    .await;
    assert!(status.is_success(), "register failed: {status} {body:?}");

    let (status, body) = send(
        app,
        post_json(
            "/api/v1/auth/login",
            json!({"email": email, "password": password}),
        ),
    )
    .await;
    assert!(status.is_success(), "login failed: {status} {body:?}");
    let d = &body["data"];
    (
        d["access_token"].as_str().unwrap().to_string(),
        d["refresh_token"].as_str().unwrap().to_string(),
    )
}

pub(crate) async fn create_admin(pool: &rust_blog::db::Pool) -> String {
    let hash = rust_blog::services::auth::hash_password("AdminPass123!").unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let sql = rust_blog::db::dialect::translate(
        "INSERT INTO users (id, email, username, password_hash, role) VALUES (?, ?, ?, ?, 'admin')",
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind("admin@test.com")
        .bind("testadmin")
        .bind(&hash)
        .execute(pool)
        .await
        .unwrap();
    id
}

pub(crate) async fn create_author(pool: &rust_blog::db::Pool) -> String {
    let hash = rust_blog::services::auth::hash_password("AuthorPass123!").unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let sql = rust_blog::db::dialect::translate(
        "INSERT INTO users (id, email, username, password_hash, role) VALUES (?, ?, ?, ?, 'author')",
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind("author@test.com")
        .bind("testauthor")
        .bind(&hash)
        .execute(pool)
        .await
        .unwrap();
    id
}

pub(crate) async fn create_published_post(app: &mut axum::Router, token: &str) -> String {
    let (_, body) = send(
        app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Test Post", "content": "content", "status": "published"}),
            token,
        ),
    )
    .await;
    body["data"]["slug"].as_str().unwrap().to_string()
}

#[path = "api/api_token.rs"]
mod api_token;
#[path = "api/audit.rs"]
mod audit;
#[path = "api/auth.rs"]
mod auth;
#[path = "api/category.rs"]
mod category;
#[path = "api/comment.rs"]
mod comment;
#[path = "api/cron.rs"]
mod cron;
#[path = "api/health.rs"]
mod health;
#[path = "api/media.rs"]
mod media;
#[path = "api/options.rs"]
mod options;
#[path = "api/plugin.rs"]
mod plugin;
#[path = "api/post.rs"]
mod post;
#[path = "api/rbac.rs"]
mod rbac;
#[path = "api/rss.rs"]
mod rss;
#[path = "api/sse.rs"]
mod sse;
#[path = "api/stats.rs"]
mod stats;
#[path = "api/tag.rs"]
mod tag;
#[path = "api/tenant_admin.rs"]
mod tenant_admin;
#[path = "api/tenant_e2e.rs"]
mod tenant_e2e;
#[path = "api/user.rs"]
mod user;
#[path = "api/webhook.rs"]
mod webhook;
#[path = "api/workflow.rs"]
mod workflow;

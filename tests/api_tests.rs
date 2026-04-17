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
    auth as h_auth, category as h_cat, comment as h_cmt, cron as h_cron, health as h_health,
    media as h_media, options as h_options, plugin as h_plugin, post as h_post, rbac as h_rbac,
    rss as h_rss, sse as h_sse, stats as h_stats, tag as h_tag, tenant as h_tenant, user as h_user,
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

fn test_config() -> AppConfig {
    AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        env: "test".into(),
        database_url: "sqlite::memory:".into(),
        db_pool_size: 1,
        jwt_secret: "test-secret-key-at-least-32-characters-long".into(),
        jwt_access_expires: 900,
        jwt_refresh_expires: 604800,
        upload_dir: std::env::temp_dir()
            .join("hello-axum-test-uploads")
            .to_string_lossy()
            .into(),
        max_upload_size: 5242880,
        static_dir: "./static".into(),
        base_url: "http://localhost:9000".into(),
        cors_origins: None,
        tls_cert_path: None,
        tls_key_path: None,
        plugin_dir: None,
        plugin_hot_reload: false,
        plugin_max_memory_mb: 32,
        plugin_default_timeout_ms: 5000,
        plugin_disabled: vec![],
        plugin_vfs_root: "./plugins-data".into(),
        plugin_vfs_max_file_size: 1048576,
        plugin_vfs_max_total_size: 10485760,
        log_dir: "./logs".into(),
        log_max_files: 7,
        rate_limit_global_max: 60,
        rate_limit_global_window: 60,
        rate_limit_register_max: 5,
        rate_limit_register_window: 3600,
        rate_limit_login_max: 10,
        rate_limit_login_window: 60,
        rate_limit_comment_max: 3,
        rate_limit_comment_window: 60,
        worker_enabled: false,
        worker_concurrency: 1,
        worker_poll_interval_ms: 500,
        worker_default_max_attempts: 3,
        worker_cron_tick_ms: 60000,
        cron_seed_enabled: false,
        cron_schedules: vec![],
        cron_log_retention_days: 30,
        search_engine: "none".into(),
        search_index_dir: "./data/search_index".into(),
        content_type_dir: "./content_types".into(),
        timezone: "UTC".into(),
        extension_dir: "./__nonexistent_extensions__".into(),
        protected_tables: rust_blog::config::app::default_protected_tables(),
    }
}

async fn test_pool() -> rust_blog::db::Pool {
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
        pool
    }
}

async fn test_app() -> (axum::Router, AppState) {
    let pool = test_pool().await;
    let config = Arc::new(test_config());
    let state = AppState {
        pool: pool.clone(),
        config: config.clone(),
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
        extension_manager: rust_blog::extension::manager::ExtensionManager::new(
            Arc::new(rust_blog::content_type::ContentTypeRegistry::new()),
            PluginManager::new(config.clone()).await,
            pool.clone(),
            &config,
        )
        .await,
        extension_service: Arc::new(rust_blog::extension::service::ExtensionService::new(
            pool.clone(),
        )),
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
            "/admin/extensions",
            get(rust_blog::extension::handler::list),
        )
        .route(
            "/admin/extensions/{id}",
            get(rust_blog::extension::handler::get)
                .delete(rust_blog::extension::handler::uninstall),
        )
        .route(
            "/admin/extensions/{id}/enable",
            http_post(rust_blog::extension::handler::enable),
        )
        .route(
            "/admin/extensions/{id}/disable",
            http_post(rust_blog::extension::handler::disable),
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

async fn send(app: &mut axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

async fn send_raw(app: &mut axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

fn post_json(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn post_json_auth(path: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn put_json_auth(path: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn get_req(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

fn get_auth(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn delete_auth(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn make_token(user_id: &str, role: &str) -> String {
    rust_blog::services::auth::generate_access_token_for_test(user_id, role)
}

async fn register_and_login(
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

async fn create_admin(pool: &rust_blog::db::Pool) -> String {
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

async fn create_author(pool: &rust_blog::db::Pool) -> String {
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

async fn create_published_post(app: &mut axum::Router, token: &str) -> String {
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

// ── health ───────────────────────────────────────────────────────

mod health {
    use super::*;

    #[tokio::test]
    async fn health_returns_ok() {
        let (mut app, _) = test_app().await;
        let (status, body): (StatusCode, Value) = send(&mut app, get_req("/health")).await;
        assert!(status.is_success());
        assert_eq!(body["data"]["status"], "ok");
        assert_eq!(body["data"]["db"], "ok");
    }
}

// ── auth ─────────────────────────────────────────────────────────

mod auth {
    use super::*;

    #[tokio::test]
    async fn register_success() {
        let (mut app, _) = test_app().await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json(
                "/api/v1/auth/register",
                json!({"email": "reg@test.com", "username": "reguser", "password": "Password123"}),
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["email"], "reg@test.com");
        assert_eq!(body["data"]["username"], "reguser");
        assert_eq!(body["data"]["role"], "reader");
    }

    #[tokio::test]
    async fn register_duplicate_email() {
        let (mut app, _) = test_app().await;
        let req_body =
            json!({"email": "dup@test.com", "username": "dup1", "password": "Password123"});
        let (s, _): (StatusCode, Value) =
            send(&mut app, post_json("/api/v1/auth/register", req_body)).await;
        assert!(s.is_success());

        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json(
                "/api/v1/auth/register",
                json!({"email": "dup@test.com", "username": "dup2", "password": "Password123"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], 40900);
    }

    #[tokio::test]
    async fn register_validation_errors() {
        let (mut app, _) = test_app().await;
        let cases = vec![
            json!({"email": "bad", "username": "user", "password": "Password123"}),
            json!({"email": "ok@test.com", "username": "a", "password": "Password123"}),
            json!({"email": "ok@test.com", "username": "user", "password": "short"}),
            json!({"email": "", "username": "", "password": ""}),
        ];
        for case in cases {
            let (status, body): (StatusCode, Value) =
                send(&mut app, post_json("/api/v1/auth/register", case)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400: {body:?}");
            assert_eq!(body["code"], 40000);
        }
    }

    #[tokio::test]
    async fn login_success() {
        let (mut app, _) = test_app().await;
        let (access, refresh) =
            register_and_login(&mut app, "login@test.com", "loginuser", "Password123").await;
        assert!(!access.is_empty());
        assert!(!refresh.is_empty());
    }

    #[tokio::test]
    async fn login_wrong_password() {
        let (mut app, _) = test_app().await;
        let _ = register_and_login(&mut app, "lwp@test.com", "lwpuser", "Password123").await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json(
                "/api/v1/auth/login",
                json!({"email": "lwp@test.com", "password": "Wrong123"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], 40100);
    }

    #[tokio::test]
    async fn login_nonexistent_user() {
        let (mut app, _) = test_app().await;
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            post_json(
                "/api/v1/auth/login",
                json!({"email": "none@test.com", "password": "Password123"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_token_success() {
        let (mut app, _) = test_app().await;
        let (_, refresh) =
            register_and_login(&mut app, "refresh@test.com", "refreshuser", "Password123").await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json("/api/v1/auth/refresh", json!({"refresh_token": refresh})),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["code"], 0);
        assert!(body["data"]["access_token"].is_string());
        assert!(body["data"]["refresh_token"].is_string());
    }

    #[tokio::test]
    async fn refresh_token_invalid() {
        let (mut app, _) = test_app().await;
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            post_json(
                "/api/v1/auth/refresh",
                json!({"refresh_token": "bad-token"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_token_rotation() {
        let (mut app, _) = test_app().await;
        let (_, r1) = register_and_login(&mut app, "rot@test.com", "rotuser", "Password123").await;

        let (_, body): (StatusCode, Value) = send(
            &mut app,
            post_json("/api/v1/auth/refresh", json!({"refresh_token": r1})),
        )
        .await;
        let r2 = body["data"]["refresh_token"].as_str().unwrap().to_string();

        let (s, _): (StatusCode, Value) = send(
            &mut app,
            post_json("/api/v1/auth/refresh", json!({"refresh_token": r1})),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "旧 token 应已失效");

        let (s, _): (StatusCode, Value) = send(
            &mut app,
            post_json("/api/v1/auth/refresh", json!({"refresh_token": r2})),
        )
        .await;
        assert!(s.is_success(), "新 token 应可用");
    }

    #[tokio::test]
    async fn logout_success() {
        let (mut app, _) = test_app().await;
        let (access, _) =
            register_and_login(&mut app, "lo@test.com", "louser", "Password123").await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/auth/logout", json!({}), &access),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["code"], 0);
    }

    #[tokio::test]
    async fn logout_without_token() {
        let (mut app, _) = test_app().await;
        let (status, _): (StatusCode, Value) =
            send(&mut app, post_json("/api/v1/auth/logout", json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

// ── user ─────────────────────────────────────────────────────────

mod user {
    use super::*;

    #[tokio::test]
    async fn get_me_success() {
        let (mut app, _) = test_app().await;
        let (access, _) =
            register_and_login(&mut app, "me@test.com", "meuser", "Password123").await;
        let (status, body): (StatusCode, Value) =
            send(&mut app, get_auth("/api/v1/users/me", &access)).await;
        assert!(status.is_success());
        assert_eq!(body["data"]["email"], "me@test.com");
    }

    #[tokio::test]
    async fn get_me_unauthorized() {
        let (mut app, _) = test_app().await;
        let (status, _): (StatusCode, Value) = send(&mut app, get_req("/api/v1/users/me")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn update_me_success() {
        let (mut app, _) = test_app().await;
        let (access, _) =
            register_and_login(&mut app, "upd@test.com", "upduser", "Password123").await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            put_json_auth(
                "/api/v1/users/me",
                json!({"username": "newname", "bio": "hello", "website": "https://example.com"}),
                &access,
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["username"], "newname");
        assert_eq!(body["data"]["bio"], "hello");
    }

    #[tokio::test]
    async fn change_password_success() {
        let (mut app, _) = test_app().await;
        let (access, _) =
            register_and_login(&mut app, "cpw@test.com", "cpwuser", "OldPass123").await;
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            put_json_auth(
                "/api/v1/users/me/password",
                json!({"old_password": "OldPass123", "new_password": "NewPass456"}),
                &access,
            ),
        )
        .await;
        assert!(status.is_success());

        let (s, _): (StatusCode, Value) = send(
            &mut app,
            post_json(
                "/api/v1/auth/login",
                json!({"email": "cpw@test.com", "password": "NewPass456"}),
            ),
        )
        .await;
        assert!(s.is_success(), "新密码应可登录");
    }

    #[tokio::test]
    async fn change_password_wrong_old() {
        let (mut app, _) = test_app().await;
        let (access, _) =
            register_and_login(&mut app, "bpw@test.com", "bpwuser", "Password123").await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            put_json_auth(
                "/api/v1/users/me/password",
                json!({"old_password": "Wrong123", "new_password": "NewPass456"}),
                &access,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], 40000);
    }

    #[tokio::test]
    async fn get_user_by_id() {
        let (mut app, state) = test_app().await;
        let _ = register_and_login(&mut app, "pub@test.com", "pubuser", "Password123").await;
        let user_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE email = 'pub@test.com'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        let (status, body): (StatusCode, Value) =
            send(&mut app, get_req(&format!("/api/v1/users/{user_id}"))).await;
        assert!(status.is_success());
        assert_eq!(body["data"]["email"], "pub@test.com");
    }

    #[tokio::test]
    async fn get_user_not_found() {
        let (mut app, _) = test_app().await;
        let fake = uuid::Uuid::now_v7().to_string();
        let (status, _): (StatusCode, Value) =
            send(&mut app, get_req(&format!("/api/v1/users/{fake}"))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_users_admin_only() {
        let (mut app, state) = test_app().await;
        let admin_id = create_admin(&state.pool).await;
        let admin_token = make_token(&admin_id, "admin");
        let (reader_tok, _) =
            register_and_login(&mut app, "reader@test.com", "reader", "Password123").await;

        let (s, _): (StatusCode, Value) =
            send(&mut app, get_auth("/api/v1/users", &reader_tok)).await;
        assert_eq!(s, StatusCode::FORBIDDEN);

        let (status, body): (StatusCode, Value) =
            send(&mut app, get_auth("/api/v1/users", &admin_token)).await;
        assert!(status.is_success());
        assert!(body["data"]["items"].is_array());
        assert!(body["data"]["total"].is_number());
    }
}

// ── category ─────────────────────────────────────────────────────

mod category {
    use super::*;

    async fn setup() -> (axum::Router, AppState, String) {
        let (mut app, state) = test_app().await;
        let id = create_author(&state.pool).await;
        let tok = make_token(&id, "author");
        // warm up router
        let _: (StatusCode, Value) = send(&mut app, get_req("/api/v1/categories")).await;
        (app, state, tok)
    }

    #[tokio::test]
    async fn list_empty() {
        let (mut app, _, _) = setup().await;
        let (status, body): (StatusCode, Value) =
            send(&mut app, get_req("/api/v1/categories")).await;
        assert!(status.is_success());
        assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn create_success() {
        let (mut app, _, tok) = setup().await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json_auth(
                "/api/v1/categories",
                json!({"name": "Rust", "description": "desc"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["name"], "Rust");
        assert_eq!(body["data"]["slug"], "rust");
    }

    #[tokio::test]
    async fn create_requires_author() {
        let (mut app, _) = test_app().await;
        let (tok, _) = register_and_login(&mut app, "catr@test.com", "catr", "Password123").await;
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/categories", json!({"name": "X"}), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn update_success() {
        let (mut app, _, tok) = setup().await;
        let (_, b): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/categories", json!({"name": "Orig"}), &tok),
        )
        .await;
        let id = b["data"]["id"].as_str().unwrap();
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/categories/{id}"),
                json!({"name": "Upd", "description": "d"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["name"], "Upd");
    }

    #[tokio::test]
    async fn delete_success() {
        let (mut app, _, tok) = setup().await;
        let (_, b): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/categories", json!({"name": "Del"}), &tok),
        )
        .await;
        let id = b["data"]["id"].as_str().unwrap();
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            delete_auth(&format!("/api/v1/categories/{id}"), &tok),
        )
        .await;
        assert!(status.is_success());
    }

    #[tokio::test]
    async fn create_validation() {
        let (mut app, _, tok) = setup().await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/categories", json!({"name": ""}), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], 40000);
    }
}

// ── tag ──────────────────────────────────────────────────────────

mod tag {
    use super::*;

    async fn setup() -> (axum::Router, AppState, String) {
        let (mut app, state) = test_app().await;
        let id = create_author(&state.pool).await;
        let tok = make_token(&id, "author");
        let _: (StatusCode, Value) = send(&mut app, get_req("/api/v1/tags")).await;
        (app, state, tok)
    }

    #[tokio::test]
    async fn list_empty() {
        let (mut app, _, _) = setup().await;
        let (status, body): (StatusCode, Value) = send(&mut app, get_req("/api/v1/tags")).await;
        assert!(status.is_success());
        assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn create_success() {
        let (mut app, _, tok) = setup().await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/tags", json!({"name": "rust"}), &tok),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["name"], "rust");
        assert_eq!(body["data"]["slug"], "rust");
    }

    #[tokio::test]
    async fn create_requires_author() {
        let (mut app, _) = test_app().await;
        let (tok, _) = register_and_login(&mut app, "tagr@test.com", "tagr", "Password123").await;
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/tags", json!({"name": "t"}), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn delete_success() {
        let (mut app, _, tok) = setup().await;
        let (_, b): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/tags", json!({"name": "delme"}), &tok),
        )
        .await;
        let id = b["data"]["id"].as_str().unwrap();
        let (status, _): (StatusCode, Value) =
            send(&mut app, delete_auth(&format!("/api/v1/tags/{id}"), &tok)).await;
        assert!(status.is_success());
    }

    #[tokio::test]
    async fn delete_not_found() {
        let (mut app, _, tok) = setup().await;
        let fake = uuid::Uuid::now_v7().to_string();
        let (status, _): (StatusCode, Value) =
            send(&mut app, delete_auth(&format!("/api/v1/tags/{fake}"), &tok)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

// ── post ─────────────────────────────────────────────────────────

mod post {
    use super::*;

    #[allow(dead_code)]
    struct Ctx {
        app: axum::Router,
        state: AppState,
        tok: String,
        author_id: String,
        cat_id: String,
        tag_id: String,
    }

    async fn setup() -> Ctx {
        let (mut app, state) = test_app().await;
        let author_id = create_author(&state.pool).await;
        let tok = make_token(&author_id, "author");

        let (_, cb): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/categories", json!({"name": "Tech"}), &tok),
        )
        .await;
        let cat_id = cb["data"]["id"].as_str().unwrap().to_string();

        let (_, tb): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/tags", json!({"name": "rust"}), &tok),
        )
        .await;
        let tag_id = tb["data"]["id"].as_str().unwrap().to_string();

        Ctx {
            app,
            state,
            tok,
            author_id,
            cat_id,
            tag_id,
        }
    }

    #[tokio::test]
    async fn create_success() {
        let mut c = setup().await;
        let (status, body): (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth(
                "/api/v1/posts",
                json!({
                    "title": "Hello Axum",
                    "content": "# Hello\n**markdown**",
                    "status": "published",
                    "category_id": c.cat_id,
                    "tag_ids": [c.tag_id],
                }),
                &c.tok,
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["title"], "Hello Axum");
        assert_eq!(body["data"]["status"], "published");
        assert!(body["data"]["slug"].is_string());
        assert!(body["data"]["html_content"].is_string());
        assert_eq!(body["data"]["tags"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn create_requires_author() {
        let (mut app, _) = test_app().await;
        let (tok, _) = register_and_login(&mut app, "pr@test.com", "pruser", "Password123").await;
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            post_json_auth("/api/v1/posts", json!({"title": "T", "content": "C"}), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_validation() {
        let mut c = setup().await;
        let (status, body): (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth("/api/v1/posts", json!({"title": "", "content": ""}), &c.tok),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], 40000);
    }

    #[tokio::test]
    async fn get_by_slug() {
        let mut c = setup().await;
        let slug = create_published_post(&mut c.app, &c.tok).await;
        let (status, body): (StatusCode, Value) =
            send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
        assert!(status.is_success());
        assert_eq!(body["data"]["title"], "Test Post");
    }

    #[tokio::test]
    async fn view_count_increments() {
        let mut c = setup().await;
        let slug = create_published_post(&mut c.app, &c.tok).await;
        let (_, b1): (StatusCode, Value) =
            send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
        assert_eq!(b1["data"]["view_count"], 1);
        let (_, b2): (StatusCode, Value) =
            send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
        assert_eq!(b2["data"]["view_count"], 2);
    }

    #[tokio::test]
    async fn get_not_found() {
        let (mut app, _) = test_app().await;
        let (status, _): (StatusCode, Value) = send(&mut app, get_req("/api/v1/posts/nope")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_paginated() {
        let mut c = setup().await;
        for i in 1..=5u8 {
            let _: (StatusCode, Value) = send(
                &mut c.app,
                post_json_auth(
                    "/api/v1/posts",
                    json!({"title": format!("P{i}"), "content": "c", "status": "published"}),
                    &c.tok,
                ),
            )
            .await;
        }
        let (status, body): (StatusCode, Value) =
            send(&mut c.app, get_req("/api/v1/posts?page=1&page_size=3")).await;
        assert!(status.is_success());
        assert_eq!(body["data"]["items"].as_array().unwrap().len(), 3);
        assert!(body["data"]["total"].as_i64().unwrap() >= 5);
        assert_eq!(body["data"]["page"], 1);
        assert_eq!(body["data"]["page_size"], 3);
    }

    #[tokio::test]
    async fn search() {
        let mut c = setup().await;
        let _: (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": "Rust Tips", "content": "Learn Rust", "status": "published"}),
                &c.tok,
            ),
        )
        .await;
        let _: (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": "Go Tips", "content": "Learn Go", "status": "published"}),
                &c.tok,
            ),
        )
        .await;
        let (_, body): (StatusCode, Value) =
            send(&mut c.app, get_req("/api/v1/posts?q=Rust")).await;
        let items = body["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "Rust Tips");
    }

    #[tokio::test]
    async fn update_success() {
        let mut c = setup().await;
        let slug = create_published_post(&mut c.app, &c.tok).await;
        let (status, body): (StatusCode, Value) = send(
            &mut c.app,
            put_json_auth(
                &format!("/api/v1/posts/{slug}"),
                json!({"title": "Updated", "status": "published"}),
                &c.tok,
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["title"], "Updated");
    }

    #[tokio::test]
    async fn update_not_owner_forbidden() {
        let mut c = setup().await;
        let slug = create_published_post(&mut c.app, &c.tok).await;
        let (other, _) =
            register_and_login(&mut c.app, "other@test.com", "otheruser", "Password123").await;
        let (status, _): (StatusCode, Value) = send(
            &mut c.app,
            put_json_auth(
                &format!("/api/v1/posts/{slug}"),
                json!({"title": "Hack"}),
                &other,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn delete_success() {
        let mut c = setup().await;
        let slug = create_published_post(&mut c.app, &c.tok).await;
        let (status, _): (StatusCode, Value) = send(
            &mut c.app,
            delete_auth(&format!("/api/v1/posts/{slug}"), &c.tok),
        )
        .await;
        assert!(status.is_success());
        let (s, _): (StatusCode, Value) =
            send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn admin_can_delete_others() {
        let mut c = setup().await;
        let slug = create_published_post(&mut c.app, &c.tok).await;
        let admin_id = create_admin(&c.state.pool).await;
        let admin_tok = make_token(&admin_id, "admin");
        let (status, _): (StatusCode, Value) = send(
            &mut c.app,
            delete_auth(&format!("/api/v1/posts/{slug}"), &admin_tok),
        )
        .await;
        assert!(status.is_success());
    }

    #[tokio::test]
    async fn filter_by_category() {
        let mut c = setup().await;
        let (_, cb): (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth("/api/v1/categories", json!({"name": "Other"}), &c.tok),
        )
        .await;
        let other_cat = cb["data"]["id"].as_str().unwrap();

        let _: (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": "InTech", "content": "c", "status": "published", "category_id": c.cat_id}),
                &c.tok,
            ),
        )
        .await;
        let _: (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": "InOther", "content": "c", "status": "published", "category_id": other_cat}),
                &c.tok,
            ),
        )
        .await;

        let (_, body): (StatusCode, Value) = send(
            &mut c.app,
            get_req(&format!("/api/v1/posts?category_id={}", c.cat_id)),
        )
        .await;
        let items = body["data"]["items"].as_array().unwrap();
        assert!(items.iter().all(|p| p["category_id"] == c.cat_id));
    }
}

// ── comment ──────────────────────────────────────────────────────

mod comment {
    use super::*;

    async fn setup_with_post() -> (axum::Router, AppState, String, String) {
        let (mut app, state) = test_app().await;
        let author_id = create_author(&state.pool).await;
        let tok = make_token(&author_id, "author");
        let slug = create_published_post(&mut app, &tok).await;
        (app, state, tok, slug)
    }

    #[tokio::test]
    async fn guest_comment_success() {
        let (mut app, _, _, slug) = setup_with_post().await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json(
                &format!("/api/v1/posts/{slug}/comments"),
                json!({"content": "Nice!", "nickname": "Guest1", "email": "g@test.com"}),
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["content"], "Nice!");
        assert_eq!(body["data"]["nickname"], "Guest1");
        assert!(body["data"]["author_id"].is_null());
    }

    #[tokio::test]
    async fn guest_comment_requires_nickname() {
        let (mut app, _, _, slug) = setup_with_post().await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json(
                &format!("/api/v1/posts/{slug}/comments"),
                json!({"content": "no nick"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], 40000);
    }

    #[tokio::test]
    async fn authed_comment_success() {
        let (mut app, _, _, slug) = setup_with_post().await;
        let (tok, _) = register_and_login(&mut app, "cmtr@test.com", "cmtr", "Password123").await;
        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json_auth(
                &format!("/api/v1/posts/{slug}/comments/authed"),
                json!({"content": "Auth comment"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["content"], "Auth comment");
        assert!(body["data"]["author_id"].is_string());
    }

    #[tokio::test]
    async fn nested_comment() {
        let (mut app, state, _, slug) = setup_with_post().await;
        let (_, b1): (StatusCode, Value) = send(
            &mut app,
            post_json(
                &format!("/api/v1/posts/{slug}/comments"),
                json!({"content": "Parent", "nickname": "G1"}),
            ),
        )
        .await;
        let pid = b1["data"]["id"].as_str().unwrap();

        let approve_sql = rust_blog::db::dialect::translate(
            "UPDATE comments SET status = 'approved' WHERE id = ?",
        );
        sqlx::query(&approve_sql)
            .bind(pid)
            .execute(&state.pool)
            .await
            .unwrap();

        let (status, body): (StatusCode, Value) = send(
            &mut app,
            post_json(
                &format!("/api/v1/posts/{slug}/comments"),
                json!({"content": "Reply", "nickname": "G2", "parent_id": pid}),
            ),
        )
        .await;
        assert!(status.is_success(), "{status} {body:?}");
        assert_eq!(body["data"]["parent_id"], pid);
    }

    #[tokio::test]
    async fn list_comments() {
        let (mut app, state, _, slug) = setup_with_post().await;
        let _: (StatusCode, Value) = send(
            &mut app,
            post_json(
                &format!("/api/v1/posts/{slug}/comments"),
                json!({"content": "C1", "nickname": "A"}),
            ),
        )
        .await;
        let _: (StatusCode, Value) = send(
            &mut app,
            post_json(
                &format!("/api/v1/posts/{slug}/comments"),
                json!({"content": "C2", "nickname": "B"}),
            ),
        )
        .await;

        sqlx::query("UPDATE comments SET status = 'approved' WHERE status = 'pending'")
            .execute(&state.pool)
            .await
            .unwrap();

        let (status, body): (StatusCode, Value) =
            send(&mut app, get_req(&format!("/api/v1/posts/{slug}/comments"))).await;
        assert!(status.is_success());
        let items = body["data"]["items"].as_array().unwrap();
        assert!(items.len() >= 2);
    }

    #[tokio::test]
    async fn delete_own_comment() {
        let (mut app, _, _, slug) = setup_with_post().await;
        let (tok, _) =
            register_and_login(&mut app, "delc@test.com", "delcuser", "Password123").await;
        let (_, b): (StatusCode, Value) = send(
            &mut app,
            post_json_auth(
                &format!("/api/v1/posts/{slug}/comments/authed"),
                json!({"content": "mine"}),
                &tok,
            ),
        )
        .await;
        let cid = b["data"]["id"].as_str().unwrap();
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            delete_auth(&format!("/api/v1/comments/{cid}"), &tok),
        )
        .await;
        assert!(status.is_success());
    }

    #[tokio::test]
    async fn delete_not_owner_forbidden() {
        let (mut app, _, _, slug) = setup_with_post().await;
        let (t1, _) = register_and_login(&mut app, "own@test.com", "own", "Password123").await;
        let (t2, _) = register_and_login(&mut app, "str@test.com", "str", "Password123").await;
        let (_, b): (StatusCode, Value) = send(
            &mut app,
            post_json_auth(
                &format!("/api/v1/posts/{slug}/comments/authed"),
                json!({"content": "x"}),
                &t1,
            ),
        )
        .await;
        let cid = b["data"]["id"].as_str().unwrap();
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            delete_auth(&format!("/api/v1/comments/{cid}"), &t2),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn update_status_admin() {
        let (mut app, state, _, slug) = setup_with_post().await;
        let _: (StatusCode, Value) = send(
            &mut app,
            post_json(
                &format!("/api/v1/posts/{slug}/comments"),
                json!({"content": "mod me", "nickname": "G"}),
            ),
        )
        .await;

        let cid: String = sqlx::query_scalar("SELECT id FROM comments WHERE content = 'mod me'")
            .fetch_one(&state.pool)
            .await
            .unwrap();

        let admin_id = create_admin(&state.pool).await;
        let admin_tok = make_token(&admin_id, "admin");
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/comments/{cid}/status"),
                json!({"status": "approved"}),
                &admin_tok,
            ),
        )
        .await;
        assert!(status.is_success());
    }

    #[tokio::test]
    async fn update_status_requires_admin() {
        let (mut app, _, _, _) = setup_with_post().await;
        let (tok, _) = register_and_login(&mut app, "na@test.com", "nauser", "Password123").await;
        let fake = uuid::Uuid::now_v7().to_string();
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/comments/{fake}/status"),
                json!({"status": "approved"}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
}

// ── media ────────────────────────────────────────────────────────

mod media {
    use super::*;

    #[tokio::test]
    async fn upload_requires_auth() {
        let (mut app, _) = test_app().await;
        let boundary = "----b";
        let body_str = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"t.png\"\r\n\r\nx\r\n--{boundary}--\r\n"
        );
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/media/upload")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body_str))
            .unwrap();
        let (status, _): (StatusCode, Value) = send(&mut app, req).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn upload_success() {
        let (mut app, state) = test_app().await;
        let author_id = create_author(&state.pool).await;
        let tok = make_token(&author_id, "author");

        let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
        let png_header = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        let body_bytes = {
            let mut v = Vec::new();
            v.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            v.extend_from_slice(
                b"Content-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\n",
            );
            v.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
            v.extend_from_slice(png_header);
            v.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            v
        };

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/media/upload")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(header::AUTHORIZATION, format!("Bearer {tok}"))
            .body(Body::from(body_bytes))
            .unwrap();

        let (status, body): (StatusCode, Value) = send(&mut app, req).await;
        assert!(status.is_success(), "upload failed: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert!(body["data"]["id"].is_string());
        assert!(body["data"]["url"].is_string());
    }

    #[tokio::test]
    async fn list_requires_auth() {
        let (mut app, _) = test_app().await;
        let (status, _): (StatusCode, Value) = send(&mut app, get_req("/api/v1/media")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_success() {
        let (mut app, state) = test_app().await;
        let author_id = create_author(&state.pool).await;
        let tok = make_token(&author_id, "author");
        let (status, body): (StatusCode, Value) =
            send(&mut app, get_auth("/api/v1/media", &tok)).await;
        assert!(status.is_success());
        assert!(body["data"]["items"].is_array());
    }

    #[tokio::test]
    async fn delete_not_found() {
        let (mut app, state) = test_app().await;
        let author_id = create_author(&state.pool).await;
        let tok = make_token(&author_id, "author");
        let fake = uuid::Uuid::now_v7().to_string();
        let (status, _): (StatusCode, Value) = send(
            &mut app,
            delete_auth(&format!("/api/v1/media/{fake}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

// ── rss ──────────────────────────────────────────────────────────

mod rss {
    use super::*;

    #[tokio::test]
    async fn feed_empty() {
        let (mut app, _) = test_app().await;
        let (status, bytes) = send_raw(&mut app, get_req("/feed.xml")).await;
        assert!(status.is_success());
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("<rss"));
        assert!(body.contains("</rss>"));
    }

    #[tokio::test]
    async fn feed_with_posts() {
        let (mut app, state) = test_app().await;
        let author_id = create_author(&state.pool).await;
        let tok = make_token(&author_id, "author");
        let _: (StatusCode, Value) = send(
            &mut app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": "RSS Post", "content": "content", "status": "published"}),
                &tok,
            ),
        )
        .await;
        let (status, bytes) = send_raw(&mut app, get_req("/feed.xml")).await;
        assert!(status.is_success());
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("RSS Post"));
    }
}

// ── cron ──────────────────────────────────────────────────────────

mod cron {
    use super::*;

    async fn cron_app() -> (axum::Router, AppState) {
        test_app().await
    }

    #[tokio::test]
    async fn list_returns_empty() {
        let admin_id = create_admin_helper().await;
        let (mut app, _) = cron_app().await;
        let tok = make_token(&admin_id, "admin");
        let (status, body) = send(&mut app, get_auth("/api/v1/admin/crons", &tok)).await;
        assert!(status.is_success());
        assert_eq!(body["code"], 0);
        assert!(body["data"]["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_and_get() {
        let admin_id = create_admin_helper().await;
        let (mut app, _) = cron_app().await;
        let tok = make_token(&admin_id, "admin");

        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/crons",
                json!({
                    "label": "Test Sitemap",
                    "job_type": "generate_sitemap",
                    "cron_expr": "0 0 */6 * * *",
                    "enabled": true
                }),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["label"], "Test Sitemap");
        assert_eq!(body["data"]["job_type"], "generate_sitemap");
        assert!(body["data"]["enabled"].as_bool().unwrap());

        let id = body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/crons/{id}"), &tok),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["data"]["id"], id);
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let admin_id = create_admin_helper().await;
        let (mut app, _) = cron_app().await;
        let tok = make_token(&admin_id, "admin");

        let (_, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/crons",
                json!({
                    "label": "Original",
                    "job_type": "my_task",
                    "cron_expr": "0 0 * * * *",
                    "enabled": true
                }),
                &tok,
            ),
        )
        .await;
        let id = body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/crons/{id}"),
                json!({"label": "Updated", "enabled": false}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["data"]["label"], "Updated");
        assert!(!body["data"]["enabled"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn toggle_enables_and_disables() {
        let admin_id = create_admin_helper().await;
        let (mut app, _) = cron_app().await;
        let tok = make_token(&admin_id, "admin");

        let (_, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/crons",
                json!({
                    "label": "Toggle Test",
                    "job_type": "test_task",
                    "cron_expr": "0 0 * * * *",
                    "enabled": true
                }),
                &tok,
            ),
        )
        .await;
        let id = body["data"]["id"].as_str().unwrap();

        let (status, _) = send(
            &mut app,
            post_json_auth(
                &format!("/api/v1/admin/crons/{id}/toggle"),
                json!({"enabled": false}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());

        let (_, body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/crons/{id}"), &tok),
        )
        .await;
        assert!(!body["data"]["enabled"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn delete_removes_schedule() {
        let admin_id = create_admin_helper().await;
        let (mut app, _) = cron_app().await;
        let tok = make_token(&admin_id, "admin");

        let (_, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/crons",
                json!({
                    "label": "To Delete",
                    "job_type": "delete_me",
                    "cron_expr": "0 0 * * * *",
                    "enabled": true
                }),
                &tok,
            ),
        )
        .await;
        let id = body["data"]["id"].as_str().unwrap();

        let (status, _) = send(
            &mut app,
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/admin/crons/{id}"))
                .header(header::AUTHORIZATION, format!("Bearer {tok}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert!(status.is_success());

        let (_, list_body) = send(&mut app, get_auth("/api/v1/admin/crons", &tok)).await;
        let items = list_body["data"]["items"].as_array().unwrap();
        assert!(items.iter().all(|s| s["id"] != id));
    }

    #[tokio::test]
    async fn logs_returns_empty_initially() {
        let admin_id = create_admin_helper().await;
        let (mut app, _) = cron_app().await;
        let tok = make_token(&admin_id, "admin");

        let (status, body) = send(&mut app, get_auth("/api/v1/admin/crons/logs", &tok)).await;
        assert!(status.is_success());
        assert!(body["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn validation_rejects_empty_label() {
        let admin_id = create_admin_helper().await;
        let (mut app, _) = cron_app().await;
        let tok = make_token(&admin_id, "admin");

        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/crons",
                json!({
                    "label": "",
                    "job_type": "test",
                    "cron_expr": "0 0 * * * *",
                    "enabled": true
                }),
                &tok,
            ),
        )
        .await;
        assert!(!status.is_success() || body["code"] != 0);
    }

    async fn create_admin_helper() -> String {
        let pool = test_pool().await;
        create_admin(&pool).await
    }
}

mod sse {
    use super::*;

    #[tokio::test]
    async fn sse_endpoint_returns_event_stream() {
        let (app, _) = test_app().await;
        let req = Request::builder()
            .uri("/api/v1/events")
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap()),
            Some("text/event-stream")
        );
        assert!(resp.headers().get("cache-control").is_some());
    }

    #[tokio::test]
    async fn sse_endpoint_with_filter_param() {
        let (app, _) = test_app().await;
        let req = Request::builder()
            .uri("/api/v1/events?filter=PostCreated,CommentCreated")
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap()),
            Some("text/event-stream")
        );
    }
}

mod tenant_e2e {
    use super::*;

    async fn create_tenant_in_db(pool: &rust_blog::db::Pool, id: &str, name: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT OR IGNORE INTO tenants (id, name, config, status, created_at, updated_at) VALUES (?, ?, '{}', 'active', ?, ?)"
        )
        .bind(id).bind(name).bind(&now).bind(&now)
        .execute(pool).await.unwrap();
    }

    async fn create_user_in_tenant(
        pool: &rust_blog::db::Pool,
        id: &str,
        email: &str,
        username: &str,
        role: &str,
        tenant_id: &str,
    ) {
        let hash = rust_blog::services::auth::hash_password("TestPass123!").unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let sql = rust_blog::db::dialect::translate(
            "INSERT INTO users (id, tenant_id, email, username, password_hash, role, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(id)
            .bind(tenant_id)
            .bind(email)
            .bind(username)
            .bind(&hash)
            .bind(role)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn create_published_post_in_tenant(
        pool: &rust_blog::db::Pool,
        id: &str,
        slug: &str,
        title: &str,
        author_id: &str,
        tenant_id: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let sql = rust_blog::db::dialect::translate(
            "INSERT INTO posts (id, tenant_id, title, slug, content, excerpt, status, author_id, created_at, updated_at) VALUES (?, ?, ?, ?, 'content', 'excerpt', 'published', ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(id)
            .bind(tenant_id)
            .bind(title)
            .bind(slug)
            .bind(author_id)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
    }

    fn login_with_tenant(email: &str, password: &str, tenant_id: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header("X-Tenant-ID", tenant_id)
            .body(Body::from(
                serde_json::to_string(&json!({"email": email, "password": password})).unwrap(),
            ))
            .unwrap()
    }

    fn get_with_tenant(path: &str, tenant_id: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(path)
            .header("X-Tenant-ID", tenant_id)
            .body(Body::empty())
            .unwrap()
    }

    fn get_auth_tenant(path: &str, token: &str, tenant_id: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("X-Tenant-ID", tenant_id)
            .body(Body::empty())
            .unwrap()
    }

    async fn do_login(
        app: &mut axum::Router,
        email: &str,
        password: &str,
        tenant_id: &str,
    ) -> String {
        let (status, body) = send(app, login_with_tenant(email, password, tenant_id)).await;
        assert!(
            status.is_success(),
            "login failed for {email} tenant={tenant_id}: {status} {body:?}"
        );
        body["data"]["access_token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn tenant_user_sees_own_data_only() {
        let (mut app, state) = test_app().await;
        let pool = &state.pool;

        create_tenant_in_db(pool, "tenant_a", "Tenant A").await;
        create_tenant_in_db(pool, "tenant_b", "Tenant B").await;

        let author_a_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_a_id,
            "author_a@tenant.test",
            "author_a",
            "author",
            "tenant_a",
        )
        .await;

        let author_b_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_b_id,
            "author_b@tenant.test",
            "author_b",
            "author",
            "tenant_b",
        )
        .await;

        let token_a = do_login(&mut app, "author_a@tenant.test", "TestPass123!", "tenant_a").await;
        let token_b = do_login(&mut app, "author_b@tenant.test", "TestPass123!", "tenant_b").await;

        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": "Post from A", "content": "content a", "status": "published"}),
                &token_a,
            ),
        )
        .await;
        assert!(status.is_success(), "create post a: {status} {body:?}");

        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": "Post from B", "content": "content b", "status": "published"}),
                &token_b,
            ),
        )
        .await;
        assert!(status.is_success(), "create post b: {status} {body:?}");

        let (status, body) = send(&mut app, get_auth("/api/v1/posts", &token_a)).await;
        assert!(status.is_success(), "author_a list: {status} {body:?}");
        let items = body["data"]["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            1,
            "author_a should see 1 post, got {}",
            items.len()
        );
        assert_eq!(items[0]["title"], "Post from A");

        let (status, body) = send(&mut app, get_auth("/api/v1/posts", &token_b)).await;
        assert!(status.is_success(), "author_b list: {status} {body:?}");
        let items = body["data"]["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            1,
            "author_b should see 1 post, got {}",
            items.len()
        );
        assert_eq!(items[0]["title"], "Post from B");
    }

    #[tokio::test]
    async fn admin_without_header_sees_all() {
        let (mut app, state) = test_app().await;
        let pool = &state.pool;

        create_tenant_in_db(pool, "tenant_a", "Tenant A").await;
        create_tenant_in_db(pool, "tenant_b", "Tenant B").await;

        let admin_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &admin_id,
            "admin_all@tenant.test",
            "admin_all",
            "admin",
            "tenant_a",
        )
        .await;

        let author_a_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_a_id,
            "author_ta@tenant.test",
            "author_ta",
            "author",
            "tenant_a",
        )
        .await;

        let author_b_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_b_id,
            "author_tb@tenant.test",
            "author_tb",
            "author",
            "tenant_b",
        )
        .await;

        create_published_post_in_tenant(
            pool,
            &uuid::Uuid::now_v7().to_string(),
            "post-tenant-a",
            "Post in Tenant A",
            &author_a_id,
            "tenant_a",
        )
        .await;

        create_published_post_in_tenant(
            pool,
            &uuid::Uuid::now_v7().to_string(),
            "post-tenant-b",
            "Post in Tenant B",
            &author_b_id,
            "tenant_b",
        )
        .await;

        let token = do_login(
            &mut app,
            "admin_all@tenant.test",
            "TestPass123!",
            "tenant_a",
        )
        .await;

        let (status, body) = send(&mut app, get_auth("/api/v1/posts", &token)).await;
        assert!(status.is_success(), "admin list: {status} {body:?}");
        let total = body["data"]["total"].as_i64().unwrap();
        assert_eq!(
            total, 2,
            "admin without header should see 2 posts, got {total}"
        );
    }

    #[tokio::test]
    async fn admin_switches_tenant_with_header() {
        let (mut app, state) = test_app().await;
        let pool = &state.pool;

        create_tenant_in_db(pool, "tenant_a", "Tenant A").await;
        create_tenant_in_db(pool, "tenant_b", "Tenant B").await;

        let admin_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &admin_id,
            "admin_switch@tenant.test",
            "admin_switch",
            "admin",
            "default",
        )
        .await;

        let author_a_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_a_id,
            "author_sw@tenant.test",
            "author_sw",
            "author",
            "tenant_a",
        )
        .await;

        create_published_post_in_tenant(
            pool,
            &uuid::Uuid::now_v7().to_string(),
            "post-switch-a",
            "Post in Tenant A for Switch",
            &author_a_id,
            "tenant_a",
        )
        .await;

        let token = do_login(
            &mut app,
            "admin_switch@tenant.test",
            "TestPass123!",
            "default",
        )
        .await;

        let (status, body) = send(
            &mut app,
            get_auth_tenant("/api/v1/posts", &token, "tenant_a"),
        )
        .await;
        assert!(status.is_success(), "admin tenant_a: {status} {body:?}");
        let total = body["data"]["total"].as_i64().unwrap();
        assert_eq!(
            total, 1,
            "admin with tenant_a header should see 1 post, got {total}"
        );

        let (status, body) = send(
            &mut app,
            get_auth_tenant("/api/v1/posts", &token, "tenant_b"),
        )
        .await;
        assert!(status.is_success(), "admin tenant_b: {status} {body:?}");
        let total = body["data"]["total"].as_i64().unwrap();
        assert_eq!(
            total, 0,
            "admin with tenant_b header should see 0 posts, got {total}"
        );
    }

    #[tokio::test]
    async fn public_api_scoped_by_tenant_header() {
        let (mut app, state) = test_app().await;
        let pool = &state.pool;

        create_tenant_in_db(pool, "tenant_a", "Tenant A").await;
        create_tenant_in_db(pool, "tenant_b", "Tenant B").await;

        let author_a_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_a_id,
            "author_pub_a@tenant.test",
            "author_pub_a",
            "author",
            "tenant_a",
        )
        .await;

        let author_b_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_b_id,
            "author_pub_b@tenant.test",
            "author_pub_b",
            "author",
            "tenant_b",
        )
        .await;

        create_published_post_in_tenant(
            pool,
            &uuid::Uuid::now_v7().to_string(),
            "public-post-a",
            "Public Post A",
            &author_a_id,
            "tenant_a",
        )
        .await;

        create_published_post_in_tenant(
            pool,
            &uuid::Uuid::now_v7().to_string(),
            "public-post-b",
            "Public Post B",
            &author_b_id,
            "tenant_b",
        )
        .await;

        let (status, body) = send(&mut app, get_with_tenant("/api/v1/posts", "tenant_a")).await;
        assert!(status.is_success(), "public tenant_a: {status} {body:?}");
        let total = body["data"]["total"].as_i64().unwrap();
        assert_eq!(
            total, 1,
            "public with tenant_a should see 1 post, got {total}"
        );

        let (status, body) = send(&mut app, get_with_tenant("/api/v1/posts", "tenant_b")).await;
        assert!(status.is_success(), "public tenant_b: {status} {body:?}");
        let total = body["data"]["total"].as_i64().unwrap();
        assert_eq!(
            total, 1,
            "public with tenant_b should see 1 post, got {total}"
        );

        let (status, body) = send(&mut app, get_req("/api/v1/posts")).await;
        assert!(status.is_success(), "public no header: {status} {body:?}");
        let total = body["data"]["total"].as_i64().unwrap();
        assert_eq!(
            total, 0,
            "public without header should see default tenant (0 posts), got {total}"
        );
    }

    #[tokio::test]
    async fn cross_tenant_post_not_accessible() {
        let (mut app, state) = test_app().await;
        let pool = &state.pool;

        create_tenant_in_db(pool, "tenant_a", "Tenant A").await;
        create_tenant_in_db(pool, "tenant_b", "Tenant B").await;

        let author_a_id = uuid::Uuid::now_v7().to_string();
        create_user_in_tenant(
            pool,
            &author_a_id,
            "author_cross@tenant.test",
            "author_cross",
            "author",
            "tenant_a",
        )
        .await;

        let post_slug = "cross-tenant-post";
        create_published_post_in_tenant(
            pool,
            &uuid::Uuid::now_v7().to_string(),
            post_slug,
            "Cross Tenant Post",
            &author_a_id,
            "tenant_a",
        )
        .await;

        let (status, body) = send(
            &mut app,
            get_with_tenant(&format!("/api/v1/posts/{post_slug}"), "tenant_a"),
        )
        .await;
        assert!(
            status.is_success(),
            "same tenant should succeed: {status} {body:?}"
        );

        let (status, _body) = send(
            &mut app,
            get_with_tenant(&format!("/api/v1/posts/{post_slug}"), "tenant_b"),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "cross-tenant access should return 404"
        );
    }
}

// ── webhook ──────────────────────────────────────────────────────────

mod webhook_tests {
    use super::*;

    async fn setup_admin() -> (axum::Router, String) {
        let admin_id = uuid::Uuid::now_v7().to_string();
        let token = make_token(&admin_id, "admin");
        let (app, _) = test_app().await;
        (app, token)
    }

    #[tokio::test]
    async fn list_empty() {
        let (mut app, tok) = setup_admin().await;
        let (status, body) = send(&mut app, get_auth("/api/v1/admin/webhooks", &tok)).await;
        assert!(status.is_success(), "list: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["total"], 0);
        assert!(body["data"]["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_success() {
        let (mut app, tok) = setup_admin().await;
        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({
                    "url": "https://example.com/hook",
                    "events": ["post.created", "post.updated"],
                    "description": "test hook"
                }),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "create: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["url"], "https://example.com/hook");
        assert!(body["data"]["secret"].as_str().unwrap().len() > 0);
        assert_eq!(body["data"]["enabled"], true);
        assert!(body["data"]["id"].as_str().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn create_default_enabled_true() {
        let (mut app, tok) = setup_admin().await;
        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({
                    "url": "https://example.com/default-enabled",
                    "events": ["*"]
                }),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["data"]["enabled"], true);
    }

    #[tokio::test]
    async fn create_with_enabled_false() {
        let (mut app, tok) = setup_admin().await;
        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({
                    "url": "https://example.com/disabled",
                    "events": ["post.created"],
                    "enabled": false
                }),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["data"]["enabled"], false);
    }

    #[tokio::test]
    async fn create_validation_empty_url() {
        let (mut app, tok) = setup_admin().await;
        let (status, _body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "", "events": ["post.created"]}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "empty url should fail");
    }

    #[tokio::test]
    async fn create_validation_empty_events() {
        let (mut app, tok) = setup_admin().await;
        let (status, _body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com", "events": []}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "empty events should fail");
    }

    #[tokio::test]
    async fn create_requires_admin() {
        let (mut app, _state) = test_app().await;
        let author_id = uuid::Uuid::now_v7().to_string();
        let tok = make_token(&author_id, "author");
        let (status, _body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com", "events": ["*"]}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "author should be forbidden");
    }

    #[tokio::test]
    async fn get_by_id() {
        let (mut app, tok) = setup_admin().await;
        let (_, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com/get", "events": ["comment.created"]}),
                &tok,
            ),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
        )
        .await;
        assert!(status.is_success(), "get: {status} {body:?}");
        assert_eq!(body["data"]["id"], id);
        assert_eq!(body["data"]["url"], "https://example.com/get");
    }

    #[tokio::test]
    async fn get_not_found() {
        let (mut app, tok) = setup_admin().await;
        let fake_id = uuid::Uuid::now_v7().to_string();
        let (status, _body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/webhooks/{fake_id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let (mut app, tok) = setup_admin().await;
        let (_, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com/old", "events": ["post.created"]}),
                &tok,
            ),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/webhooks/{id}"),
                json!({"url": "https://example.com/new", "description": "updated desc", "enabled": false}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "update: {status} {body:?}");
        assert_eq!(body["data"]["url"], "https://example.com/new");
        assert_eq!(body["data"]["description"], "updated desc");
        assert_eq!(body["data"]["enabled"], false);
    }

    #[tokio::test]
    async fn update_events() {
        let (mut app, tok) = setup_admin().await;
        let (_, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com/ev", "events": ["post.created"]}),
                &tok,
            ),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/webhooks/{id}"),
                json!({"events": ["post.created", "post.updated", "comment.created"]}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        let events: Vec<String> =
            serde_json::from_str(body["data"]["events"].as_str().unwrap()).unwrap();
        assert_eq!(events.len(), 3);
        assert!(events.contains(&"post.created".to_string()));
        assert!(events.contains(&"post.updated".to_string()));
        assert!(events.contains(&"comment.created".to_string()));
    }

    #[tokio::test]
    async fn update_partial_only_description() {
        let (mut app, tok) = setup_admin().await;
        let (_, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com/partial", "events": ["*"]}),
                &tok,
            ),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();
        let original_url = create_body["data"]["url"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/webhooks/{id}"),
                json!({"description": "partial update"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["data"]["url"], original_url);
        assert_eq!(body["data"]["description"], "partial update");
    }

    #[tokio::test]
    async fn delete_success() {
        let (mut app, tok) = setup_admin().await;
        let (_, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com/del", "events": ["*"]}),
                &tok,
            ),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            delete_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
        )
        .await;
        assert!(status.is_success(), "delete: {status} {body:?}");

        let (status, _body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "should be deleted");
    }

    #[tokio::test]
    async fn delete_not_found() {
        let (mut app, tok) = setup_admin().await;
        let fake_id = uuid::Uuid::now_v7().to_string();
        let (status, _body) = send(
            &mut app,
            delete_auth(&format!("/api/v1/admin/webhooks/{fake_id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn update_not_found() {
        let (mut app, tok) = setup_admin().await;
        let fake_id = uuid::Uuid::now_v7().to_string();
        let (status, _body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/webhooks/{fake_id}"),
                json!({"url": "https://example.com/ghost"}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_with_pagination() {
        let (mut app, tok) = setup_admin().await;
        for i in 0..5 {
            send(
                &mut app,
                post_json_auth(
                    "/api/v1/admin/webhooks",
                    json!({"url": format!("https://example.com/page-{i}"), "events": ["*"]}),
                    &tok,
                ),
            )
            .await;
        }

        let (status, body) = send(
            &mut app,
            get_auth("/api/v1/admin/webhooks?page=1&page_size=3", &tok),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["data"]["items"].as_array().unwrap().len(), 3);
        assert_eq!(body["data"]["total"], 5);

        let (status, body2) = send(
            &mut app,
            get_auth("/api/v1/admin/webhooks?page=2&page_size=3", &tok),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body2["data"]["items"].as_array().unwrap().len(), 2);
        assert_eq!(body2["data"]["total"], 5);
    }

    #[tokio::test]
    async fn secret_is_hex_64_chars() {
        let (mut app, tok) = setup_admin().await;
        let (_, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": "https://example.com/secret", "events": ["*"]}),
                &tok,
            ),
        )
        .await;
        let secret = body["data"]["secret"].as_str().unwrap();
        assert_eq!(secret.len(), 64, "secret should be 64 hex chars (32 bytes)");
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn full_lifecycle_create_read_update_delete() {
        let (mut app, tok) = setup_admin().await;

        let (status, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({
                    "url": "https://example.com/lifecycle",
                    "events": ["post.created"],
                    "description": "lifecycle test"
                }),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        let id = create_body["data"]["id"].as_str().unwrap().to_string();
        let secret = create_body["data"]["secret"].as_str().unwrap().to_string();

        let (status, get_body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(get_body["data"]["secret"], secret.as_str());

        let (status, update_body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/webhooks/{id}"),
                json!({"enabled": false, "url": "https://example.com/lifecycle-v2"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(
            update_body["data"]["url"],
            "https://example.com/lifecycle-v2"
        );
        assert_eq!(update_body["data"]["enabled"], false);
        assert_eq!(
            update_body["data"]["secret"],
            secret.as_str(),
            "secret should not change on update"
        );

        let (status, list_body) = send(&mut app, get_auth("/api/v1/admin/webhooks", &tok)).await;
        assert!(status.is_success());
        assert_eq!(list_body["data"]["total"], 1);

        let (status, _) = send(
            &mut app,
            delete_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
        )
        .await;
        assert!(status.is_success());

        let (status, _) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, list_body) = send(&mut app, get_auth("/api/v1/admin/webhooks", &tok)).await;
        assert!(status.is_success());
        assert_eq!(list_body["data"]["total"], 0);
    }
}

// ── options ──────────────────────────────────────────────────────────

mod options_tests {
    use super::*;

    fn admin_token() -> String {
        let id = uuid::Uuid::now_v7().to_string();
        make_token(&id, "admin")
    }

    #[tokio::test]
    async fn public_options_empty() {
        let (mut app, _) = test_app().await;
        let (status, body) = send(&mut app, get_req("/api/v1/options/public")).await;
        assert!(status.is_success(), "public options: {status} {body:?}");
        assert_eq!(body["code"], 0);
    }

    #[tokio::test]
    async fn list_options_empty() {
        let (mut app, _) = test_app().await;
        let tok = admin_token();
        let (status, body) = send(&mut app, get_auth("/api/v1/admin/options", &tok)).await;
        assert!(status.is_success(), "list options: {status} {body:?}");
        assert_eq!(body["code"], 0);
    }

    #[tokio::test]
    async fn set_get_delete_option() {
        let (mut app, _) = test_app().await;
        let tok = admin_token();

        let key = format!("test.{}", uuid::Uuid::now_v7().to_string());

        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/options/{key}"),
                json!({"value": "Test Value"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "set option: {status} {body:?}");

        let (status, _body) = send(
            &mut app,
            delete_auth(&format!("/api/v1/admin/options/{key}"), &tok),
        )
        .await;
        assert!(status.is_success(), "delete option: {status}");
    }

    #[tokio::test]
    async fn batch_update_options() {
        let (mut app, _) = test_app().await;
        let tok = admin_token();

        let key1 = format!("test.{}", &uuid::Uuid::now_v7().to_string()[..8]);
        let key2 = format!("test.{}", &uuid::Uuid::now_v7().to_string()[..8]);

        let (status, body) = send(
            &mut app,
            put_json_auth(
                "/api/v1/admin/options",
                json!({"options": {key1: "Val1", key2: "Val2"}}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "batch update: {status} {body:?}");
    }
}

// ── rbac ──────────────────────────────────────────────────────────

mod rbac_tests {
    use super::*;

    async fn admin_token() -> String {
        let pool = test_pool().await;
        let id = create_admin(&pool).await;
        make_token(&id, "admin")
    }

    #[tokio::test]
    async fn list_roles() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, body) = send(&mut app, get_auth("/api/v1/admin/rbac/roles", &tok)).await;
        assert!(status.is_success(), "list roles: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert!(body["data"]["items"].is_array());
    }

    #[tokio::test]
    async fn create_role_returns_in_list() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let role_name = format!("editor-{}", &uuid::Uuid::now_v7().to_string()[..8]);

        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/rbac/roles",
                json!({"name": role_name, "description": "Editor role"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "create role: {status} {body:?}");
        assert_eq!(body["data"]["name"], role_name);
        let role_id = body["data"]["id"].as_str().unwrap().to_string();

        let (status, body) = send(&mut app, get_auth("/api/v1/admin/rbac/roles", &tok)).await;
        assert!(status.is_success(), "list roles: {status} {body:?}");
        let items = body["data"]["items"].as_array().unwrap();
        let found = items.iter().any(|r| r["id"] == role_id);
        assert!(found, "created role should appear in list");
    }

    #[tokio::test]
    async fn update_role() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;

        let (_, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/rbac/roles",
                json!({"name": format!("mod-{}", &uuid::Uuid::now_v7().to_string()[..8])}),
                &tok,
            ),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let new_name = format!("super-{}", &uuid::Uuid::now_v7().to_string()[..8]);
        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/rbac/roles/{id}"),
                json!({"name": new_name, "description": "updated"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "update role: {status} {body:?}");
        assert_eq!(body["data"]["name"], new_name);
    }

    #[tokio::test]
    async fn delete_role() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;

        let role_name = format!("del-{}", &uuid::Uuid::now_v7().to_string()[..8]);
        let (_, create_body) = send(
            &mut app,
            post_json_auth("/api/v1/admin/rbac/roles", json!({"name": role_name}), &tok),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let (status, _) = send(
            &mut app,
            delete_auth(&format!("/api/v1/admin/rbac/roles/{id}"), &tok),
        )
        .await;
        assert!(status.is_success(), "delete role: {status}");

        let (status, body) = send(&mut app, get_auth("/api/v1/admin/rbac/roles", &tok)).await;
        assert!(status.is_success());
        let items = body["data"]["items"].as_array().unwrap();
        let found = items.iter().any(|r| r["id"] == id);
        assert!(!found, "deleted role should not appear in list");
    }

    #[tokio::test]
    async fn set_and_get_permissions() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;

        let (_, create_body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/rbac/roles",
                json!({"name": format!("perm-{}", &uuid::Uuid::now_v7().to_string()[..8])}),
                &tok,
            ),
        )
        .await;
        let role_id = create_body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/rbac/roles/{role_id}/permissions"),
                json!({"permissions": [
                    {"action": "read", "subject": "posts"},
                    {"action": "write", "subject": "comments", "conditions": {"own": "true"}}
                ]}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "set perms: {status} {body:?}");

        let (status, body) = send(
            &mut app,
            get_auth(
                &format!("/api/v1/admin/rbac/roles/{role_id}/permissions"),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "get perms: {status} {body:?}");
        let perms = body["data"].as_array().unwrap();
        assert_eq!(perms.len(), 2);
    }
}

// ── stats ──────────────────────────────────────────────────────────

mod stats_tests {
    use super::*;

    #[tokio::test]
    async fn overview_empty() {
        let (mut app, _) = test_app().await;
        let (status, body) = send(&mut app, get_req("/api/v1/admin/stats")).await;
        assert!(status.is_success(), "stats overview: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert!(body["data"]["total_posts"].is_number());
        assert!(body["data"]["total_users"].is_number());
    }

    #[tokio::test]
    async fn content_stats() {
        let (mut app, _) = test_app().await;
        let (status, body) = send(&mut app, get_req("/api/v1/admin/stats/content/posts")).await;
        assert!(status.is_success(), "content stats: {status} {body:?}");
        assert_eq!(body["code"], 0);
    }

    #[tokio::test]
    async fn trends() {
        let (mut app, _) = test_app().await;
        let (status, body) = send(
            &mut app,
            get_req("/api/v1/admin/stats/trends?table=posts&days=7"),
        )
        .await;
        assert!(status.is_success(), "trends: {status} {body:?}");
        assert_eq!(body["code"], 0);
    }
}

// ── plugin ──────────────────────────────────────────────────────────

mod plugin_tests {
    use super::*;

    async fn admin_token() -> String {
        let pool = test_pool().await;
        let id = create_admin(&pool).await;
        make_token(&id, "admin")
    }

    #[tokio::test]
    async fn list_plugins_empty() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, body) = send(&mut app, get_auth("/api/v1/admin/plugins", &tok)).await;
        assert!(status.is_success(), "list plugins: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["total"], 0);
    }

    #[tokio::test]
    async fn get_nonexistent_plugin() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let fake_id = "nonexistent-plugin";
        let (status, _body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/plugins/{fake_id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn enable_nonexistent_plugin() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, _body) = send(
            &mut app,
            post_json_auth("/api/v1/admin/plugins/ghost/enable", json!({}), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn disable_nonexistent_plugin() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, _body) = send(
            &mut app,
            post_json_auth("/api/v1/admin/plugins/ghost/disable", json!({}), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn reload_nonexistent_plugin() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, _body) = send(
            &mut app,
            post_json_auth("/api/v1/admin/plugins/ghost/reload", json!({}), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn remove_nonexistent_plugin_is_idempotent() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, _body) =
            send(&mut app, delete_auth("/api/v1/admin/plugins/ghost", &tok)).await;
        assert!(status.is_success(), "remove is idempotent: {status}");
    }
}

// ── tenant admin ──────────────────────────────────────────────────

mod tenant_admin_tests {
    use super::*;

    async fn admin_token() -> String {
        let pool = test_pool().await;
        let id = create_admin(&pool).await;
        make_token(&id, "admin")
    }

    #[tokio::test]
    async fn create_and_get_tenant() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;

        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/tenants",
                json!({"name": "Acme Corp", "domain": "acme.example.com"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "create tenant: {status} {body:?}");
        assert_eq!(body["data"]["name"], "Acme Corp");
        let id = body["data"]["id"].as_str().unwrap().to_string();

        let (status, body) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/tenants/{id}"), &tok),
        )
        .await;
        assert!(status.is_success(), "get tenant: {status} {body:?}");
        assert_eq!(body["data"]["name"], "Acme Corp");
    }

    #[tokio::test]
    async fn update_tenant() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;

        let (_, create_body) = send(
            &mut app,
            post_json_auth("/api/v1/admin/tenants", json!({"name": "Original"}), &tok),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let (status, body) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/tenants/{id}"),
                json!({"name": "Updated Corp", "domain": "updated.example.com"}),
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "update tenant: {status} {body:?}");
        assert_eq!(body["data"]["name"], "Updated Corp");
    }

    #[tokio::test]
    async fn delete_tenant() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;

        let (_, create_body) = send(
            &mut app,
            post_json_auth("/api/v1/admin/tenants", json!({"name": "ToDelete"}), &tok),
        )
        .await;
        let id = create_body["data"]["id"].as_str().unwrap();

        let (status, _) = send(
            &mut app,
            delete_auth(&format!("/api/v1/admin/tenants/{id}"), &tok),
        )
        .await;
        assert!(status.is_success(), "delete tenant: {status}");

        let (status, _) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/tenants/{id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_tenants() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;

        send(
            &mut app,
            post_json_auth("/api/v1/admin/tenants", json!({"name": "Tenant A"}), &tok),
        )
        .await;
        send(
            &mut app,
            post_json_auth("/api/v1/admin/tenants", json!({"name": "Tenant B"}), &tok),
        )
        .await;

        let (status, body) = send(&mut app, get_auth("/api/v1/admin/tenants", &tok)).await;
        assert!(status.is_success(), "list tenants: {status} {body:?}");
        assert!(body["data"]["total"].as_i64().unwrap() >= 2);
    }

    #[tokio::test]
    async fn get_tenant_not_found() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let fake_id = uuid::Uuid::now_v7().to_string();
        let (status, _) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/tenants/{fake_id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn update_tenant_not_found() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let fake_id = uuid::Uuid::now_v7().to_string();
        let (status, _) = send(
            &mut app,
            put_json_auth(
                &format!("/api/v1/admin/tenants/{fake_id}"),
                json!({"name": "Ghost"}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_tenant_not_found_is_idempotent() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let fake_id = uuid::Uuid::now_v7().to_string();
        let (status, _) = send(
            &mut app,
            delete_auth(&format!("/api/v1/admin/tenants/{fake_id}"), &tok),
        )
        .await;
        assert!(status.is_success(), "delete tenant is idempotent: {status}");
    }
}

// ── audit ──────────────────────────────────────────────────────────

mod audit_tests {
    use super::*;

    async fn admin_token() -> String {
        let pool = test_pool().await;
        let id = create_admin(&pool).await;
        make_token(&id, "admin")
    }

    #[tokio::test]
    async fn list_audit_empty() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, body) = send(&mut app, get_auth("/api/v1/admin/audit", &tok)).await;
        assert!(status.is_success(), "list audit: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["total"], 0);
    }

    #[tokio::test]
    async fn get_audit_not_found() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let fake_id = uuid::Uuid::now_v7().to_string();
        let (status, _) = send(
            &mut app,
            get_auth(&format!("/api/v1/admin/audit/{fake_id}"), &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_audit_with_filter() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, body) = send(
            &mut app,
            get_auth(
                "/api/v1/admin/audit?action=create&page=1&page_size=10",
                &tok,
            ),
        )
        .await;
        assert!(status.is_success(), "audit filter: {status} {body:?}");
        assert_eq!(body["code"], 0);
    }

    #[tokio::test]
    async fn audit_pagination() {
        let (mut app, _) = test_app().await;
        let tok = admin_token().await;
        let (status, body) = send(
            &mut app,
            get_auth("/api/v1/admin/audit?page=2&page_size=5", &tok),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(body["data"]["page"], 2);
        assert_eq!(body["data"]["page_size"], 5);
    }
}

// ── extension ────────────────────────────────────────────────────────

mod extension_tests {
    use super::*;

    async fn setup_admin() -> (axum::Router, String) {
        let admin_id = uuid::Uuid::now_v7().to_string();
        let token = make_token(&admin_id, "admin");
        let (app, _) = test_app().await;
        (app, token)
    }

    #[tokio::test]
    async fn list_returns_empty_when_no_extensions() {
        let (mut app, tok) = setup_admin().await;
        let (status, body) = send(&mut app, get_auth("/api/v1/admin/extensions", &tok)).await;
        assert!(status.is_success(), "list: {status} {body:?}");
        assert_eq!(body["code"], 0);
        assert!(body["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_nonexistent_returns_not_found() {
        let (mut app, tok) = setup_admin().await;
        let (status, _) = send(
            &mut app,
            get_auth("/api/v1/admin/extensions/nonexistent", &tok),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn enable_nonexistent_returns_not_found() {
        let (mut app, tok) = setup_admin().await;
        let (status, _) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/extensions/nonexistent/enable",
                json!({}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn disable_nonexistent_returns_not_found() {
        let (mut app, tok) = setup_admin().await;
        let (status, _) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/extensions/nonexistent/disable",
                json!({}),
                &tok,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn uninstall_nonexistent_returns_not_found() {
        let (mut app, tok) = setup_admin().await;
        let (status, _) = send(
            &mut app,
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/api/v1/admin/extensions/nonexistent")
                .header(header::AUTHORIZATION, format!("Bearer {tok}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"drop_tables":false}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_requires_auth() {
        let (mut app, _) = setup_admin().await;
        let (status, _) = send(
            &mut app,
            Request::get("/api/v1/admin/extensions")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

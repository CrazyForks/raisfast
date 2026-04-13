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
use rust_blog::config::app::AppConfig;
use rust_blog::handlers::{
    auth as h_auth, category as h_cat, comment as h_cmt, health as h_health, media as h_media,
    post as h_post, rss as h_rss, tag as h_tag, user as h_user,
};
use rust_blog::middleware::locale::locale_middleware;
use rust_blog::middleware::rate_limit::{
    RateLimiterSet, comment_rate_limit, global_rate_limit, login_rate_limit, register_rate_limit,
};
use rust_blog::plugins::PluginManager;
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
        plugin_dir: None,
        plugin_hot_reload: false,
        plugin_max_memory_mb: 32,
        plugin_default_timeout_ms: 5000,
        plugin_disabled: vec![],
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
        pool
    }
}

async fn test_app() -> (axum::Router, AppState) {
    let pool = test_pool().await;
    let config = test_config();
    let config = Arc::new(test_config());
    let state = AppState {
        pool,
        config: config.clone(),
        plugins: Arc::new(PluginManager::new(config).await),
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
        assert_eq!(body["data"].as_array().unwrap().len(), 0);
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
        assert_eq!(body["data"].as_array().unwrap().len(), 0);
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

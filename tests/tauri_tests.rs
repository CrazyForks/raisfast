//! Tauri 适配层集成测试
//!
//! 验证 Tauri command 背后的 service 调用链正确工作。
//! 这些测试直接调用 service 函数（与 Tauri command 调用的完全相同的函数），
//! 因为 command 层只是薄适配（参数透传 + 错误转字符串）。

use std::sync::Arc;

use rust_blog::config::app::AppConfig;
use rust_blog::content_type::repository::{ContentQuery, ContentRepository, SaveContext};
use rust_blog::content_type::schema::ContentTypeSchema;
use rust_blog::db::tenant;
use rust_blog::repositories::*;
use rust_blog::services::{auth, options, post, stats};

fn test_config() -> AppConfig {
    let mut config = AppConfig::test_defaults();
    config.database_url = "sqlite::memory:".into();
    config
}

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    for sql in [
        include_str!("../migrations/001_init.sql"),
        include_str!("../migrations/002_add_indexes.sql"),
        include_str!("../migrations/009_options.sql"),
        include_str!("../migrations/010_rbac.sql"),
        include_str!("../migrations/011_tenants.sql"),
        include_str!("../migrations/014_extensions.sql"),
        include_str!("../migrations/015_api_tokens.sql"),
        include_str!("../migrations/016_content_revisions.sql"),
        include_str!("../migrations/019_oauth.sql"),
        include_str!("../migrations/021_phone.sql"),
        include_str!("../migrations/022_email_verification.sql"),
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    tenant::invalidate_cache().await;
    pool
}

async fn create_test_user(pool: &sqlx::SqlitePool, label: &str) -> String {
    let eventbus = rust_blog::eventbus::EventBus::new(16);
    let user_repo = SqlxUserRepository::new(pool.clone());
    let req = rust_blog::handlers::dto::RegisterRequest {
        username: format!("user_{label}"),
        email: format!("{label}@test.com"),
        password: "Password123".into(),
    };
    let user = auth::register(&user_repo, &eventbus, req, None, false, pool)
        .await
        .unwrap();
    user.id
}

fn parse_todo_ct() -> ContentTypeSchema {
    let toml_str = r#"
[content_type]
name = "Todo"
singular = "todo"
plural = "todos"
table = "test_todos"
description = "测试待办"
draft_publish = false
timestamps = true

[fields.title]
type = "text"
required = true
label = "标题"

[fields.done]
type = "boolean"
default = false
label = "已完成"

[fields.priority]
type = "enum"
enum_values = ["low", "medium", "high"]
default = "medium"
label = "优先级"
"#;
    ContentTypeSchema::parse_from_str(toml_str).unwrap()
}

// ═══════════════════════════════════════════════════════════════
// Auth service 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_auth_register_service() {
    let pool = setup_pool().await;
    let user_repo = SqlxUserRepository::new(pool.clone());
    let eventbus = rust_blog::eventbus::EventBus::new(16);

    let req = rust_blog::handlers::dto::RegisterRequest {
        username: "testuser".into(),
        email: "test@example.com".into(),
        password: "Password123".into(),
    };

    let result = auth::register(&user_repo, &eventbus, req, None, false, &pool).await;

    assert!(
        result.is_ok(),
        "register should succeed: {:?}",
        result.err()
    );
    let user = result.unwrap();
    assert_eq!(user.username, "testuser");
    assert_eq!(user.email, "test@example.com");
}

#[tokio::test]
async fn tauri_auth_register_duplicate_email() {
    let pool = setup_pool().await;
    let user_repo = SqlxUserRepository::new(pool.clone());
    let eventbus = rust_blog::eventbus::EventBus::new(16);

    let req = rust_blog::handlers::dto::RegisterRequest {
        username: "user1".into(),
        email: "dup@example.com".into(),
        password: "Password123".into(),
    };
    auth::register(&user_repo, &eventbus, req, None, false, &pool)
        .await
        .unwrap();

    let req2 = rust_blog::handlers::dto::RegisterRequest {
        username: "user2".into(),
        email: "dup@example.com".into(),
        password: "Password456".into(),
    };
    let eventbus2 = rust_blog::eventbus::EventBus::new(16);
    let result = auth::register(&user_repo, &eventbus2, req2, None, false, &pool).await;
    assert!(result.is_err(), "duplicate email should fail");
}

#[tokio::test]
async fn tauri_auth_login_service() {
    let pool = setup_pool().await;
    let config = test_config();
    let user_repo = SqlxUserRepository::new(pool.clone());
    let refresh_repo = SqlxRefreshTokenRepository::new(pool.clone());
    let eventbus = rust_blog::eventbus::EventBus::new(16);
    let plugin_mgr = rust_blog::plugins::PluginManager::new(Arc::new(config.clone())).await;

    let reg_req = rust_blog::handlers::dto::RegisterRequest {
        username: "loginuser".into(),
        email: "login@example.com".into(),
        password: "Password123".into(),
    };
    auth::register(&user_repo, &eventbus, reg_req, None, false, &pool)
        .await
        .unwrap();

    let login_req = rust_blog::handlers::dto::LoginRequest {
        email: "login@example.com".into(),
        password: "Password123".into(),
    };
    let eventbus2 = rust_blog::eventbus::EventBus::new(16);
    let result = auth::login(
        &user_repo,
        &refresh_repo,
        &plugin_mgr,
        &eventbus2,
        &login_req,
        &config.jwt_secret,
        config.jwt_access_expires,
        config.jwt_refresh_expires,
        None,
        false,
    )
    .await;

    assert!(result.is_ok(), "login should succeed: {:?}", result.err());
    let login_resp = result.unwrap();
    assert!(!login_resp.access_token.is_empty());
    assert!(!login_resp.refresh_token.is_empty());
}

#[tokio::test]
async fn tauri_auth_login_wrong_password() {
    let pool = setup_pool().await;
    let config = test_config();
    let user_repo = SqlxUserRepository::new(pool.clone());
    let refresh_repo = SqlxRefreshTokenRepository::new(pool.clone());
    let eventbus = rust_blog::eventbus::EventBus::new(16);
    let plugin_mgr = rust_blog::plugins::PluginManager::new(Arc::new(config.clone())).await;

    let reg_req = rust_blog::handlers::dto::RegisterRequest {
        username: "wrongpw".into(),
        email: "wrong@example.com".into(),
        password: "Password123".into(),
    };
    auth::register(&user_repo, &eventbus, reg_req, None, false, &pool)
        .await
        .unwrap();

    let login_req = rust_blog::handlers::dto::LoginRequest {
        email: "wrong@example.com".into(),
        password: "WrongPassword".into(),
    };
    let eventbus2 = rust_blog::eventbus::EventBus::new(16);
    let result = auth::login(
        &user_repo,
        &refresh_repo,
        &plugin_mgr,
        &eventbus2,
        &login_req,
        &config.jwt_secret,
        config.jwt_access_expires,
        config.jwt_refresh_expires,
        None,
        false,
    )
    .await;

    assert!(result.is_err(), "wrong password should fail");
}

#[tokio::test]
async fn tauri_auth_get_me_service() {
    let pool = setup_pool().await;
    let user_repo = SqlxUserRepository::new(pool.clone());
    let eventbus = rust_blog::eventbus::EventBus::new(16);

    let reg_req = rust_blog::handlers::dto::RegisterRequest {
        username: "getme".into(),
        email: "getme@example.com".into(),
        password: "Password123".into(),
    };
    let user = auth::register(&user_repo, &eventbus, reg_req, None, false, &pool)
        .await
        .unwrap();

    let result = auth::get_me(&user_repo, &user.id, None).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().id, user.id);
}

// ═══════════════════════════════════════════════════════════════
// Post service 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_post_create_and_list() {
    let pool = setup_pool().await;
    let author_id = create_test_user(&pool, "author-001").await;
    let sqlx_repo = SqlxPostRepository::new(pool.clone());
    let post_repo: Arc<dyn PostRepository> =
        Arc::new(rust_blog::repositories::CachedPostRepository::new(
            sqlx_repo,
            Arc::new(rust_blog::cache::MemoryCache::new()),
            None,
        ));
    let eventbus = rust_blog::eventbus::EventBus::new(16);
    let config = test_config();
    let plugin_mgr = rust_blog::plugins::PluginManager::new(Arc::new(config.clone())).await;

    let req = rust_blog::handlers::dto::CreatePostRequest {
        title: "Test Post".into(),
        content: "Hello world".into(),
        excerpt: None,
        cover_image: None,
        status: Some("published".into()),
        category_id: None,
        tag_ids: None,
    };

    let created = post::create_post(
        post_repo.as_ref(),
        &plugin_mgr,
        &eventbus,
        &author_id,
        req,
        None,
    )
    .await
    .unwrap();

    assert_eq!(created.title, "Test Post");
    assert_eq!(created.author_id, author_id);

    let (items, total) = post::list_posts(
        post_repo.as_ref(),
        1,
        20,
        None,
        None,
        None,
        &plugin_mgr,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(items[0].title, "Test Post");
}

#[tokio::test]
async fn tauri_post_get_by_slug() {
    let pool = setup_pool().await;
    let author_id = create_test_user(&pool, "author-002").await;
    let sqlx_repo = SqlxPostRepository::new(pool.clone());
    let post_repo: Arc<dyn PostRepository> =
        Arc::new(rust_blog::repositories::CachedPostRepository::new(
            sqlx_repo,
            Arc::new(rust_blog::cache::MemoryCache::new()),
            None,
        ));
    let eventbus = rust_blog::eventbus::EventBus::new(16);
    let config = test_config();
    let plugin_mgr = rust_blog::plugins::PluginManager::new(Arc::new(config.clone())).await;

    let req = rust_blog::handlers::dto::CreatePostRequest {
        title: "Slug Test".into(),
        content: "content".into(),
        excerpt: None,
        cover_image: None,
        status: Some("published".into()),
        category_id: None,
        tag_ids: None,
    };

    let created = post::create_post(
        post_repo.as_ref(),
        &plugin_mgr,
        &eventbus,
        &author_id,
        req,
        None,
    )
    .await
    .unwrap();

    let found = post::get_post(post_repo.as_ref(), &created.slug, &plugin_mgr, None)
        .await
        .unwrap();

    assert_eq!(found.id, created.id);
    assert_eq!(found.title, "Slug Test");
}

// ═══════════════════════════════════════════════════════════════
// CMS Content Type CRUD 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_cms_create_and_list() {
    let pool = setup_pool().await;
    let ct = parse_todo_ct();
    let registry = rust_blog::content_type::ContentTypeRegistry::new();
    let config = test_config();
    registry.register(ct.clone(), &config.rule_engine, &config.builtins.reserved_route_segments()).unwrap();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext::default();
    let data = serde_json::json!({
        "title": "Buy milk",
        "done": false,
        "priority": "high"
    });

    let created = repo.create(&ct, data, None, &save_ctx).await.unwrap();
    assert_eq!(created["title"], "Buy milk");

    let query = ContentQuery {
        page: 1,
        page_size: 20,
        max_page_size: 100,
        include_private: false,
        ..Default::default()
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0]["title"], "Buy milk");
}

#[tokio::test]
async fn tauri_cms_get_by_id() {
    let pool = setup_pool().await;
    let ct = parse_todo_ct();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext::default();
    let data = serde_json::json!({"title": "Read book", "done": false});
    let created = repo.create(&ct, data, None, &save_ctx).await.unwrap();

    let id = created["id"].as_str().unwrap();
    let found = repo.find_by_id(&ct, id, None, true).await.unwrap().unwrap();
    assert_eq!(found["title"], "Read book");
}

#[tokio::test]
async fn tauri_cms_update() {
    let pool = setup_pool().await;
    let ct = parse_todo_ct();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext::default();
    let data = serde_json::json!({"title": "Original", "done": false});
    let created = repo.create(&ct, data, None, &save_ctx).await.unwrap();

    let id = created["id"].as_str().unwrap().to_string();
    let update_data = serde_json::json!({"title": "Updated", "done": true});
    repo.update(&ct, &id, update_data, None, &save_ctx)
        .await
        .unwrap();

    let found = repo.find_by_id(&ct, &id, None, true).await.unwrap().unwrap();
    assert_eq!(found["title"], "Updated");
}

#[tokio::test]
async fn tauri_cms_delete() {
    let pool = setup_pool().await;
    let ct = parse_todo_ct();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext::default();
    let data = serde_json::json!({"title": "To delete", "done": false});
    let created = repo.create(&ct, data, None, &save_ctx).await.unwrap();

    let id = created["id"].as_str().unwrap().to_string();
    repo.delete(&ct, &id, None).await.unwrap();

    let found = repo.find_by_id(&ct, &id, None, true).await.unwrap();
    assert!(found.is_none(), "deleted item should not exist");
}

#[tokio::test]
async fn tauri_cms_boolean_field_stored_as_integer() {
    let pool = setup_pool().await;
    let ct = parse_todo_ct();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext::default();
    let data = serde_json::json!({"title": "Boolean test", "done": true});
    let created = repo.create(&ct, data, None, &save_ctx).await.unwrap();

    assert_eq!(created["done"], 1);
}

#[tokio::test]
async fn tauri_cms_enum_field_validation() {
    let pool = setup_pool().await;
    let ct = parse_todo_ct();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext::default();
    let data = serde_json::json!({"title": "Enum test", "priority": "low"});
    let created = repo.create(&ct, data, None, &save_ctx).await.unwrap();

    assert_eq!(created["priority"], "low");
}

// ═══════════════════════════════════════════════════════════════
// Content Type Registry 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_registry_register_and_query() {
    let registry = rust_blog::content_type::ContentTypeRegistry::new();
    let config = test_config();
    let ct = parse_todo_ct();
    registry.register(ct, &config.rule_engine, &config.builtins.reserved_route_segments()).unwrap();

    assert!(registry.get("todo").is_some());
    assert!(registry.get_by_plural("todos").is_some());
    assert!(registry.get_by_table("test_todos").is_some());
    assert!(registry.get("nonexistent").is_none());
    assert_eq!(registry.len(), 1);
}

#[tokio::test]
async fn tauri_registry_unregister() {
    let registry = rust_blog::content_type::ContentTypeRegistry::new();
    let config = test_config();
    let ct = parse_todo_ct();
    registry.register(ct, &config.rule_engine, &config.builtins.reserved_route_segments()).unwrap();

    assert_eq!(registry.len(), 1);
    let removed = registry.unregister("todo");
    assert!(removed.is_some());
    assert_eq!(registry.len(), 0);
    assert!(registry.get("todo").is_none());
}

#[tokio::test]
async fn tauri_registry_list_all() {
    let registry = rust_blog::content_type::ContentTypeRegistry::new();
    let config = test_config();
    let ct1 = parse_todo_ct();

    let ct2_toml = r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "test_notes"
draft_publish = false

[fields.body]
type = "text"
label = "内容"
"#;
    let ct2 = ContentTypeSchema::parse_from_str(ct2_toml).unwrap();

    registry.register(ct1, &config.rule_engine, &config.builtins.reserved_route_segments()).unwrap();
    registry.register(ct2, &config.rule_engine, &config.builtins.reserved_route_segments()).unwrap();

    let all = registry.all();
    assert_eq!(all.len(), 2);
}

// ═══════════════════════════════════════════════════════════════
// Options service 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_options_set_and_get() {
    let pool = setup_pool().await;
    let repo: Arc<dyn OptionsRepository> = Arc::new(SqlxOptionsRepository::new(pool));
    let svc = options::OptionsService::new(repo).await;

    svc.set("site.title", serde_json::json!("My Blog"))
        .await
        .unwrap();

    let val = svc.get("site.title").await;
    assert_eq!(val, Some(serde_json::json!("My Blog")));
}

#[tokio::test]
async fn tauri_options_get_nonexistent() {
    let pool = setup_pool().await;
    let repo: Arc<dyn OptionsRepository> = Arc::new(SqlxOptionsRepository::new(pool));
    let svc = options::OptionsService::new(repo).await;

    let val = svc.get("nonexistent.key").await;
    assert!(val.is_none());
}

#[tokio::test]
async fn tauri_options_overwrite() {
    let pool = setup_pool().await;
    let repo: Arc<dyn OptionsRepository> = Arc::new(SqlxOptionsRepository::new(pool));
    let svc = options::OptionsService::new(repo).await;

    svc.set("key1", serde_json::json!("value1")).await.unwrap();
    svc.set("key1", serde_json::json!("value2")).await.unwrap();

    let val = svc.get("key1").await;
    assert_eq!(val, Some(serde_json::json!("value2")));
}

// ═══════════════════════════════════════════════════════════════
// Stats service 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_stats_overview() {
    let pool = setup_pool().await;
    let svc = stats::StatsService::new(pool);

    let result = svc.overview(None).await;
    assert!(result.is_ok());

    let overview = result.unwrap();
    assert!(overview.get("posts_count").is_some() || overview.get("total_posts").is_some());
}

// ═══════════════════════════════════════════════════════════════
// CMS list params 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_cms_list_with_pagination() {
    let pool = setup_pool().await;
    let ct = parse_todo_ct();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext::default();
    for i in 0..5 {
        let data = serde_json::json!({"title": format!("Todo {}", i), "done": false});
        repo.create(&ct, data, None, &save_ctx).await.unwrap();
    }

    let query = ContentQuery {
        page: 1,
        page_size: 2,
        max_page_size: 100,
        include_private: false,
        ..Default::default()
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(total, 5);

    let query2 = ContentQuery {
        page: 2,
        page_size: 2,
        max_page_size: 100,
        include_private: false,
        ..Default::default()
    };
    let (items2, _) = repo.find(&ct, query2).await.unwrap();
    assert_eq!(items2.len(), 2);

    let query3 = ContentQuery {
        page: 3,
        page_size: 2,
        max_page_size: 100,
        include_private: false,
        ..Default::default()
    };
    let (items3, _) = repo.find(&ct, query3).await.unwrap();
    assert_eq!(items3.len(), 1);
}

// ═══════════════════════════════════════════════════════════════
// SaveContext 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_save_context_auto_fill() {
    let pool = setup_pool().await;

    let ct_toml = r#"
[content_type]
name = "AutoFill"
singular = "autofill"
plural = "autofills"
table = "test_autofills"
draft_publish = false

[fields.title]
type = "text"
required = true

[fields.author_id]
type = "text"
auto_fill = "user_id"
"#;
    let ct = ContentTypeSchema::parse_from_str(ct_toml).unwrap();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let save_ctx = SaveContext {
        user_id: Some("user-123".into()),
        user_role: Some("member".into()),
        tenant_id: None,
    };

    let data = serde_json::json!({"title": "Auto fill test"});
    let created = repo.create(&ct, data, None, &save_ctx).await.unwrap();

    assert_eq!(created["author_id"], "user-123");
}

// ═══════════════════════════════════════════════════════════════
// build_app_state 测试
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn tauri_build_app_state_succeeds() {
    let mut config = AppConfig::test_defaults();
    let dir = tempfile::tempdir().unwrap();
    config.database_url = format!("sqlite:{}/test.db?mode=rwc", dir.path().display());
    config.upload_dir = format!("{}/uploads", dir.path().display());
    config.static_dir = format!("{}/static", dir.path().display());
    config.log_dir = format!("{}/logs", dir.path().display());
    config.content_type_dir = format!("{}/content_types", dir.path().display());
    config.plugin_dir = None;
    std::fs::create_dir_all(&config.content_type_dir).unwrap();

    let result = rust_blog::build_app_state(&config).await;
    assert!(
        result.is_ok(),
        "build_app_state should succeed: {:?}",
        result.err()
    );

    let state = result.unwrap();
    assert!(!state.pool.is_closed());
}

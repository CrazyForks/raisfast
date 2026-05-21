use super::*;
use raisfast::DbDriver;

async fn has_tenant_id_column(pool: &raisfast::db::Pool) -> bool {
    #[cfg(feature = "db-sqlite")]
    {
        let result: Result<(i64,), sqlx::Error> = sqlx::query_as(
            "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name = 'tenant_id'",
        )
        .fetch_one(pool)
        .await;
        result.is_ok_and(|(c,)| c > 0)
    }
    #[cfg(not(feature = "db-sqlite"))]
    {
        let _ = pool;
        false
    }
}

macro_rules! skip_without_tenant {
    ($pool:expr) => {
        if !has_tenant_id_column($pool).await {
            return;
        }
    };
}

async fn create_tenant_in_db(pool: &raisfast::db::Pool, name: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT OR IGNORE INTO tenants (name, config, status, created_at, updated_at) VALUES (?, '{}', 'active', ?, ?)"
    )
    .bind(name).bind(&now).bind(&now)
    .execute(pool).await.unwrap();
}

async fn create_user_in_tenant(
    pool: &raisfast::db::Pool,
    email: &str,
    username: &str,
    role: &str,
    tenant_id: &str,
) -> i64 {
    let hash = raisfast::services::auth::hash_password("TestPass123!").unwrap();
    let sql = format!(
        "INSERT INTO users (tenant_id, username, role, status, registered_via) VALUES ({}, {}, {}, 'active', 'email') RETURNING id",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2),
        raisfast::db::Driver::ph(3)
    );
    let int_id: i64 = sqlx::query_scalar(&sql)
        .bind(tenant_id)
        .bind(username)
        .bind(role)
        .fetch_one(pool)
        .await
        .unwrap();
    let cred_data = serde_json::json!({"password_hash": hash}).to_string();
    let cred_id = raisfast::utils::id::new_id();
    let cred_now = raisfast::utils::tz::now_utc();
    let cred_sql = format!(
        "INSERT INTO user_credentials (id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, 'email', {}, {}, 1, {}, {})",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2),
        raisfast::db::Driver::ph(3),
        raisfast::db::Driver::ph(4),
        raisfast::db::Driver::ph(5),
        raisfast::db::Driver::ph(6)
    );
    sqlx::query(&cred_sql)
        .bind(cred_id)
        .bind(int_id)
        .bind(email)
        .bind(&cred_data)
        .bind(cred_now)
        .bind(cred_now)
        .execute(pool)
        .await
        .unwrap();
    int_id
}

async fn create_published_post_in_tenant(
    pool: &raisfast::db::Pool,
    slug: &str,
    title: &str,
    author_int_id: i64,
    tenant_id: &str,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let sql = format!(
        "INSERT INTO posts (tenant_id, title, slug, content, excerpt, status, created_by, updated_by, created_at, updated_at) VALUES ({}, {}, {}, 'content', 'excerpt', 'published', {}, NULL, {}, {})",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2),
        raisfast::db::Driver::ph(3),
        raisfast::db::Driver::ph(4),
        raisfast::db::Driver::ph(5),
        raisfast::db::Driver::ph(6)
    );
    sqlx::query(&sql)
        .bind(tenant_id)
        .bind(title)
        .bind(slug)
        .bind(author_int_id)
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

async fn do_login(app: &mut axum::Router, email: &str, password: &str, tenant_id: &str) -> String {
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
    skip_without_tenant!(pool);

    create_tenant_in_db(pool, "Tenant A").await;
    create_tenant_in_db(pool, "Tenant B").await;

    create_user_in_tenant(
        pool,
        "author_a@tenant.test",
        "author_a",
        "author",
        "tenant_a",
    )
    .await;

    create_user_in_tenant(
        pool,
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
    skip_without_tenant!(pool);

    create_tenant_in_db(pool, "Tenant A").await;
    create_tenant_in_db(pool, "Tenant B").await;

    create_user_in_tenant(
        pool,
        "admin_all@tenant.test",
        "admin_all",
        "admin",
        "tenant_a",
    )
    .await;

    let author_a_int_id = create_user_in_tenant(
        pool,
        "author_ta@tenant.test",
        "author_ta",
        "author",
        "tenant_a",
    )
    .await;

    let author_b_int_id = create_user_in_tenant(
        pool,
        "author_tb@tenant.test",
        "author_tb",
        "author",
        "tenant_b",
    )
    .await;

    create_published_post_in_tenant(
        pool,
        "post-tenant-a",
        "Post in Tenant A",
        author_a_int_id,
        "tenant_a",
    )
    .await;

    create_published_post_in_tenant(
        pool,
        "post-tenant-b",
        "Post in Tenant B",
        author_b_int_id,
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
    skip_without_tenant!(pool);

    create_tenant_in_db(pool, "Tenant A").await;
    create_tenant_in_db(pool, "Tenant B").await;

    create_user_in_tenant(
        pool,
        "admin_switch@tenant.test",
        "admin_switch",
        "admin",
        "default",
    )
    .await;

    let author_a_int_id = create_user_in_tenant(
        pool,
        "author_sw@tenant.test",
        "author_sw",
        "author",
        "tenant_a",
    )
    .await;

    create_published_post_in_tenant(
        pool,
        "post-switch-a",
        "Post in Tenant A for Switch",
        author_a_int_id,
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
    skip_without_tenant!(pool);

    create_tenant_in_db(pool, "Tenant A").await;
    create_tenant_in_db(pool, "Tenant B").await;

    let author_a_int_id = create_user_in_tenant(
        pool,
        "author_pub_a@tenant.test",
        "author_pub_a",
        "author",
        "tenant_a",
    )
    .await;

    let author_b_int_id = create_user_in_tenant(
        pool,
        "author_pub_b@tenant.test",
        "author_pub_b",
        "author",
        "tenant_b",
    )
    .await;

    create_published_post_in_tenant(
        pool,
        "public-post-a",
        "Public Post A",
        author_a_int_id,
        "tenant_a",
    )
    .await;

    create_published_post_in_tenant(
        pool,
        "public-post-b",
        "Public Post B",
        author_b_int_id,
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
    skip_without_tenant!(pool);

    create_tenant_in_db(pool, "Tenant A").await;
    create_tenant_in_db(pool, "Tenant B").await;

    let author_a_int_id = create_user_in_tenant(
        pool,
        "author_cross@tenant.test",
        "author_cross",
        "author",
        "tenant_a",
    )
    .await;

    let post_slug = "cross-tenant-post";
    create_published_post_in_tenant(
        pool,
        post_slug,
        "Cross Tenant Post",
        author_a_int_id,
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

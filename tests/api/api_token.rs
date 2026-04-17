use super::*;

async fn setup() -> (axum::Router, String, rust_blog::db::Pool) {
    let (app, state) = test_app().await;
    let admin_id = create_admin(&state.pool).await;
    let tok = make_token(&admin_id, "admin");
    (app, tok, state.pool)
}

#[tokio::test]
async fn create_token_success() {
    let (mut app, tok, _) = setup().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "CI/CD", "scopes": ["read", "write"]}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let data = &body["data"];
    assert!(data["token"].as_str().unwrap().starts_with("rblog_"));
    assert_eq!(data["name"].as_str().unwrap(), "CI/CD");
}

#[tokio::test]
async fn list_tokens() {
    let (mut app, tok, _) = setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "My Token", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/tokens", &tok)).await;
    assert!(status.is_success());
    let tokens = body["data"].as_array().unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["name"].as_str().unwrap(), "My Token");
    assert!(tokens[0]["token"].is_null());
    assert_eq!(
        tokens[0]["token_prefix"].as_str().unwrap(),
        create_body["data"]["token_prefix"].as_str().unwrap()
    );
}

#[tokio::test]
async fn delete_token() {
    let (mut app, tok, _) = setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "To Delete", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(&mut app, delete_auth(&format!("/api/v1/tokens/{id}"), &tok)).await;
    assert!(status.is_success());

    let (_, body) = send(&mut app, get_auth("/api/v1/tokens", &tok)).await;
    assert!(body["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn delete_token_not_found() {
    let (mut app, tok, _) = setup().await;
    let (status, _) = send(&mut app, delete_auth("/api/v1/tokens/nonexistent", &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_token_requires_auth() {
    let (mut app, _, _) = setup().await;
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/tokens", json!({"name": "x", "scopes": ["read"]})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_token_authenticates_as_user() {
    let (mut app, tok, _) = setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Test Auth", "scopes": ["read", "write"]}),
            &tok,
        ),
    )
    .await;
    let api_token = create_body["data"]["token"].as_str().unwrap();

    let (status, body) = send(&mut app, get_auth("/api/v1/users/me", api_token)).await;
    assert!(
        status.is_success(),
        "api token auth failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["email"].as_str().unwrap(), "admin@test.com");
}

#[tokio::test]
async fn api_token_admin_scope() {
    let (mut app, tok, _) = setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Admin Token", "scopes": ["admin"]}),
            &tok,
        ),
    )
    .await;
    let api_token = create_body["data"]["token"].as_str().unwrap();

    let (status, body) = send(&mut app, get_auth("/api/v1/admin/stats", api_token)).await;
    assert!(status.is_success(), "admin scope failed: {status} {body:?}");
}

#[tokio::test]
async fn api_token_read_scope_cannot_create_post() {
    let (mut app, tok, _) = setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Read Only", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;
    let api_token = create_body["data"]["token"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "t", "content": "c", "slug": "s"}),
            api_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invalid_api_token_rejected() {
    let (mut app, _, _) = setup().await;
    let (status, _) = send(
        &mut app,
        get_auth("/api/v1/users/me", "rblog_invalid_token_here"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_token_non_owner_forbidden() {
    let (mut app, tok, pool) = setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Owned", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let reader_hash = rust_blog::services::auth::hash_password("ReaderPass123!").unwrap();
    let reader_id = uuid::Uuid::now_v7().to_string();
    let sql = rust_blog::db::dialect::translate(
        "INSERT INTO users (id, email, username, password_hash, role) VALUES (?, ?, ?, ?, 'reader')",
    );
    sqlx::query(&sql)
        .bind(&reader_id)
        .bind("reader-token@test.com")
        .bind("tokenreader")
        .bind(&reader_hash)
        .execute(&pool)
        .await
        .unwrap();
    let reader_tok = make_token(&reader_id, "reader");

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/tokens/{id}"), &reader_tok),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_delete_other_users_token() {
    let (mut app, tok, pool) = setup().await;
    let reader_hash = rust_blog::services::auth::hash_password("ReaderPass123!").unwrap();
    let reader_id = uuid::Uuid::now_v7().to_string();
    let sql = rust_blog::db::dialect::translate(
        "INSERT INTO users (id, email, username, password_hash, role) VALUES (?, ?, ?, ?, 'reader')",
    );
    sqlx::query(&sql)
        .bind(&reader_id)
        .bind("reader-admin-del@test.com")
        .bind("readeradmindel")
        .bind(&reader_hash)
        .execute(&pool)
        .await
        .unwrap();
    let reader_tok = make_token(&reader_id, "reader");

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Reader Token", "scopes": ["read"]}),
            &reader_tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(&mut app, delete_auth(&format!("/api/v1/tokens/{id}"), &tok)).await;
    assert!(status.is_success());
}

#[tokio::test]
async fn create_token_validation_empty_name() {
    let (mut app, tok, _) = setup().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422, got {status}: {body:?}"
    );
}

#[tokio::test]
async fn create_token_validation_empty_scopes() {
    let (mut app, tok, _) = setup().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Test", "scopes": []}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_token_validation_invalid_scope() {
    let (mut app, tok, _) = setup().await;
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Test", "scopes": ["superuser"]}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_token_validation_missing_body() {
    let (mut app, tok, _) = setup().await;
    let (status, _) = send(
        &mut app,
        Request::builder()
            .method("POST")
            .uri("/api/v1/tokens")
            .header(header::AUTHORIZATION, format!("Bearer {tok}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422, got {status}"
    );
}

#[tokio::test]
async fn token_deleted_cannot_authenticate() {
    let (mut app, tok, _) = setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Then Delete", "scopes": ["read", "write"]}),
            &tok,
        ),
    )
    .await;
    let api_token = create_body["data"]["token"].as_str().unwrap();
    let id = create_body["data"]["id"].as_str().unwrap();

    send(&mut app, delete_auth(&format!("/api/v1/tokens/{id}"), &tok)).await;

    let (status, _) = send(&mut app, get_auth("/api/v1/users/me", api_token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn multiple_tokens_per_user() {
    let (mut app, tok, _) = setup().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "First", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Second", "scopes": ["write"]}),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Third", "scopes": ["admin"]}),
            &tok,
        ),
    )
    .await;

    let (_, body) = send(&mut app, get_auth("/api/v1/tokens", &tok)).await;
    let tokens = body["data"].as_array().unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0]["name"].as_str().unwrap(), "Third");
    assert_eq!(tokens[1]["name"].as_str().unwrap(), "Second");
    assert_eq!(tokens[2]["name"].as_str().unwrap(), "First");
}

#[tokio::test]
async fn list_tokens_sanitized_fields() {
    let (mut app, tok, _) = setup().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Safe", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;

    let (_, body) = send(&mut app, get_auth("/api/v1/tokens", &tok)).await;
    let token = &body["data"][0];
    let obj = token.as_object().unwrap();

    assert!(obj.contains_key("id"));
    assert!(obj.contains_key("name"));
    assert!(obj.contains_key("token_prefix"));
    assert!(obj.contains_key("scopes"));
    assert!(obj.contains_key("created_at"));
    assert!(!obj.contains_key("token"), "full token must not appear");
    assert!(!obj.contains_key("token_hash"), "hash must not appear");
    assert!(!obj.contains_key("user_id"), "user_id must not appear");
}

#[tokio::test]
async fn token_with_expires_at() {
    let (mut app, tok, _) = setup().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Expiring", "scopes": ["read"], "expires_at": "2099-12-31T00:00:00+00:00"}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        body["data"]["expires_at"].as_str().unwrap(),
        "2099-12-31T00:00:00+00:00"
    );
}

#[tokio::test]
async fn token_prefix_is_first_8_chars() {
    let (mut app, tok, _) = setup().await;
    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Prefix", "scopes": ["read"]}),
            &tok,
        ),
    )
    .await;
    let full_token = body["data"]["token"].as_str().unwrap();
    let prefix = body["data"]["token_prefix"].as_str().unwrap();
    assert_eq!(prefix, &full_token[..8]);
    assert!(prefix.starts_with("rblog_"));
}

#[tokio::test]
async fn list_requires_auth() {
    let (mut app, _, _) = setup().await;
    let (status, _) = send(&mut app, get_req("/api/v1/tokens")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_requires_auth() {
    let (mut app, _, _) = setup().await;
    let (status, _) = send(
        &mut app,
        Request::builder()
            .method("DELETE")
            .uri("/api/v1/tokens/some-id")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn each_user_sees_only_own_tokens() {
    let (mut app, tok, pool) = setup().await;

    let reader_hash = rust_blog::services::auth::hash_password("ReaderPass123!").unwrap();
    let reader_id = uuid::Uuid::now_v7().to_string();
    let sql = rust_blog::db::dialect::translate(
        "INSERT INTO users (id, email, username, password_hash, role) VALUES (?, ?, ?, ?, 'reader')",
    );
    sqlx::query(&sql)
        .bind(&reader_id)
        .bind("isolation@test.com")
        .bind("isolationreader")
        .bind(&reader_hash)
        .execute(&pool)
        .await
        .unwrap();
    let reader_tok = make_token(&reader_id, "reader");

    send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Admin Token", "scopes": ["admin"]}),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "Reader Token", "scopes": ["read"]}),
            &reader_tok,
        ),
    )
    .await;

    let (_, admin_body) = send(&mut app, get_auth("/api/v1/tokens", &tok)).await;
    assert_eq!(admin_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(
        admin_body["data"][0]["name"].as_str().unwrap(),
        "Admin Token"
    );

    let (_, reader_body) = send(&mut app, get_auth("/api/v1/tokens", &reader_tok)).await;
    assert_eq!(reader_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(
        reader_body["data"][0]["name"].as_str().unwrap(),
        "Reader Token"
    );
}

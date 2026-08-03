//! Comprehensive permission system integration tests.
//!
//! Tests the full middleware pipeline: request → permission_guard → handler.
//! Covers all permission types through real HTTP requests with real tokens.

use super::*;

async fn perm_setup() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    let (iid, uid) = create_author(&state.pool).await;
    let tok = make_token(&uid, iid, raisfast::models::user::UserRole::Author);
    (app, state, tok)
}

async fn make_api_token(app: &mut axum::Router, jwt: &str, scopes: &[&str]) -> String {
    let scope_json: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
    let (_, body) = send(
        app,
        post_json_auth(
            "/api/v1/tokens",
            json!({"name": "perm-test", "scopes": scope_json}),
            jwt,
        ),
    )
    .await;
    body["data"]["token"].as_str().unwrap().to_string()
}

// ── API Token scope: positive ──

#[tokio::test]
async fn token_valid_scope_can_read() {
    let (mut app, _, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["posts:read"]).await;
    let (status, _) = send(&mut app, get_auth("/api/v1/posts", &t)).await;
    assert!(status.is_success(), "valid scope read: {status}");
}

#[tokio::test]
async fn token_valid_scope_can_create() {
    let (mut app, _, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["posts:create"]).await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title":"T","content":"C"}), &t),
    )
    .await;
    assert!(status.is_success(), "posts:create scope: {status}");
}

#[tokio::test]
async fn token_resource_wildcard() {
    let (mut app, _, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["posts:*"]).await;
    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title":"WS","content":"C"}), &t),
    )
    .await;
    assert!(status.is_success(), "posts:* create: {status}");
    let slug = body["data"]["slug"].as_str().unwrap();
    let (status, _) = send(&mut app, get_auth("/api/v1/posts", &t)).await;
    assert!(status.is_success(), "posts:* read: {status}");
    let (status, _) = send(
        &mut app,
        put_json_auth(&format!("/api/v1/posts/{slug}"), json!({"title":"Up"}), &t),
    )
    .await;
    assert!(status.is_success(), "posts:* update: {status}");
}

#[tokio::test]
async fn token_star_scope() {
    let (mut app, _, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["*"]).await;
    let (status, _) = send(&mut app, get_auth("/api/v1/posts", &t)).await;
    assert!(status.is_success(), "* scope posts read: {status}");
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/tags", json!({"name":"Star"}), &t),
    )
    .await;
    assert!(status.is_success(), "* scope tags create: {status}");
}

#[tokio::test]
async fn token_action_wildcard() {
    let (mut app, _, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["*:read"]).await;
    let (status, _) = send(&mut app, get_auth("/api/v1/posts", &t)).await;
    assert!(status.is_success(), "*:read posts: {status}");
    let (status, _) = send(&mut app, get_auth("/api/v1/categories", &t)).await;
    assert!(status.is_success(), "*:read categories: {status}");
}

// ── API Token scope: negative ──

#[tokio::test]
async fn token_wrong_action_forbidden() {
    let (mut app, _, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["posts:read"]).await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title":"T","content":"C"}), &t),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn token_wrong_resource_forbidden() {
    let (mut app, _, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["tags:create"]).await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title":"T","content":"C"}), &t),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn token_read_cannot_delete() {
    let (mut app, _, tok) = perm_setup().await;
    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title":"D","content":"C","status":"published"}),
            &tok,
        ),
    )
    .await;
    let slug = body["data"]["slug"].as_str().unwrap();
    let t = make_api_token(&mut app, &tok, &["posts:read"]).await;
    let (status, _) = send(&mut app, delete_auth(&format!("/api/v1/posts/{slug}"), &t)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ── Anonymous ──

#[tokio::test]
async fn anon_resource_action_401() {
    let (mut app, _, _) = perm_setup().await;
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/posts", json!({"title":"T","content":"C"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn anon_admin_401() {
    let (mut app, _, _) = perm_setup().await;
    let (status, _) = send(&mut app, get_req("/api/v1/admin/stats")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn anon_authed_401() {
    let (mut app, _, _) = perm_setup().await;
    let (status, _) = send(&mut app, get_req("/api/v1/tokens")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn anon_public_200() {
    let (mut app, _, _) = perm_setup().await;
    let (status, _) = send(&mut app, get_req("/api/v1/posts")).await;
    assert!(status.is_success(), "public route: {status}");
}

// ── JWT user ──

#[tokio::test]
async fn jwt_passes_resource_action() {
    let (mut app, _, tok) = perm_setup().await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title":"J","content":"C"}), &tok),
    )
    .await;
    assert!(
        status.is_success(),
        "JWT should pass resource:action: {status}"
    );
}

#[tokio::test]
async fn jwt_author_rejected_admin() {
    let (mut app, _, tok) = perm_setup().await;
    let (status, _) = send(&mut app, get_auth("/api/v1/admin/stats", &tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ── Admin ──

#[tokio::test]
async fn admin_accesses_admin_route() {
    let (mut app, state) = test_app().await;
    let (_, uid) = create_admin(&state.pool).await;
    let tok = make_token(
        &uid,
        uid.parse().unwrap(),
        raisfast::models::user::UserRole::Admin,
    );
    let (status, _) = send(&mut app, get_auth("/api/v1/admin/stats", &tok)).await;
    assert!(status.is_success(), "admin route: {status}");
}

#[tokio::test]
async fn non_admin_rejected_admin() {
    let (mut app, _, tok) = perm_setup().await;
    let (status, _) = send(&mut app, get_auth("/api/v1/admin/stats", &tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ── Fail-closed ──

#[tokio::test]
async fn malformed_scopes_fail_closed() {
    let (mut app, state, tok) = perm_setup().await;
    let t = make_api_token(&mut app, &tok, &["posts:read"]).await;
    let hash = raisfast::services::api_token::hash_token(&t);

    // Corrupt scopes in DB
    sqlx::query("UPDATE api_tokens SET scopes = ? WHERE token_hash = ?")
        .bind("{bad json")
        .bind(&hash)
        .execute(&state.pool)
        .await
        .unwrap();

    // Verify DB was updated
    let db_scopes: String =
        sqlx::query_scalar("SELECT scopes FROM api_tokens WHERE token_hash = ?")
            .bind(&hash)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(db_scopes, "{bad json", "DB should have corrupted scopes");

    // Invalidate cache
    let _ = state.cache.delete(&format!("api_token:{hash}")).await;

    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title":"T","content":"C"}), &t),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "malformed scopes should fail-closed"
    );
}

// ── Payment callback public ──

#[tokio::test]
async fn payment_callback_public() {
    let (mut app, _, _) = perm_setup().await;
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/payment/callback/none", json!({})),
    )
    .await;
    assert!(
        status != StatusCode::UNAUTHORIZED && status != StatusCode::FORBIDDEN,
        "callback should be public: {status}"
    );
}

// ── Auth config public ──

#[tokio::test]
async fn auth_config_public() {
    let (mut app, _, _) = perm_setup().await;
    let (status, _) = send(&mut app, get_req("/api/v1/auth/config")).await;
    assert!(
        status != StatusCode::UNAUTHORIZED && status != StatusCode::FORBIDDEN,
        "auth/config should be public: {status}"
    );
}

// ── Admin bypass on resource:action ──

#[tokio::test]
async fn admin_bypass_scope() {
    let (mut app, state) = test_app().await;
    let (_, uid) = create_admin(&state.pool).await;
    let jwt = make_token(
        &uid,
        uid.parse().unwrap(),
        raisfast::models::user::UserRole::Admin,
    );
    let t = make_api_token(&mut app, &jwt, &["posts:read"]).await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title":"A","content":"C"}), &t),
    )
    .await;
    assert!(status.is_success(), "admin bypass: {status}");
}

// ── Ownership policy ──

#[tokio::test]
async fn author_cannot_update_others_post() {
    let (mut app, state) = test_app().await;
    let (id, uid) = create_author(&state.pool).await;
    let tok1 = make_token(&uid, id, raisfast::models::user::UserRole::Author);
    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title":"M","content":"C","status":"published"}),
            &tok1,
        ),
    )
    .await;
    let slug = body["data"]["slug"].as_str().unwrap();
    let (tok2, _) = register_and_login(&mut app, "other@test.com", "otherusr", "Password123").await;
    let (status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"title":"Hack"}),
            &tok2,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn author_updates_own_post() {
    let (mut app, state) = test_app().await;
    let (id, uid) = create_author(&state.pool).await;
    let tok = make_token(&uid, id, raisfast::models::user::UserRole::Author);
    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title":"M","content":"C","status":"published"}),
            &tok,
        ),
    )
    .await;
    let slug = body["data"]["slug"].as_str().unwrap();
    let (status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"title":"Up"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "own post: {status}");
}

#[tokio::test]
async fn admin_updates_any_post() {
    let (mut app, state) = test_app().await;
    let (id, uid) = create_author(&state.pool).await;
    let atok = make_token(&uid, id, raisfast::models::user::UserRole::Author);
    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title":"A","content":"C","status":"published"}),
            &atok,
        ),
    )
    .await;
    let slug = body["data"]["slug"].as_str().unwrap();
    let (_, auid) = create_admin(&state.pool).await;
    let adtok = make_token(
        &auid,
        auid.parse().unwrap(),
        raisfast::models::user::UserRole::Admin,
    );
    let (status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"title":"Admin"}),
            &adtok,
        ),
    )
    .await;
    assert!(status.is_success(), "admin update: {status}");
}

#[tokio::test]
async fn admin_deletes_any_post() {
    let (mut app, state) = test_app().await;
    let (id, uid) = create_author(&state.pool).await;
    let atok = make_token(&uid, id, raisfast::models::user::UserRole::Author);
    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title":"D","content":"C","status":"published"}),
            &atok,
        ),
    )
    .await;
    let slug = body["data"]["slug"].as_str().unwrap();
    let (_, auid) = create_admin(&state.pool).await;
    let adtok = make_token(
        &auid,
        auid.parse().unwrap(),
        raisfast::models::user::UserRole::Admin,
    );
    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/posts/{slug}"), &adtok),
    )
    .await;
    assert!(status.is_success(), "admin delete: {status}");
}

use super::*;

#[tokio::test]
async fn get_me_success() {
    let (mut app, _) = test_app().await;
    let (access, _) = register_and_login(&mut app, "me@test.com", "meuser", "Password123").await;
    let (status, body): (StatusCode, Value) =
        send(&mut app, get_auth("/api/v1/users/me", &access)).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["username"], "meuser");
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
    let (access, _) = register_and_login(&mut app, "upd@test.com", "upduser", "Password123").await;
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
    let (access, _) = register_and_login(&mut app, "cpw@test.com", "cpwuser", "OldPass123").await;
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
    let (access, _) = register_and_login(&mut app, "bpw@test.com", "bpwuser", "Password123").await;
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
    let (token, _) = register_and_login(&mut app, "pub@test.com", "pubuser", "Password123").await;
    let user_id_i64: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'pubuser'")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    let user_id = user_id_i64.to_string();
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        get_auth(&format!("/api/v1/users/{user_id}"), &token),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["username"], "pubuser");
}

#[tokio::test]
async fn get_user_not_found() {
    let (mut app, _) = test_app().await;
    let (token, _) =
        register_and_login(&mut app, "notfound@test.com", "nfuser", "Password123").await;
    let fake = "9999999999999";
    let (status, _): (StatusCode, Value) =
        send(&mut app, get_auth(&format!("/api/v1/users/{fake}"), &token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_users_admin_only() {
    let (mut app, state) = test_app().await;
    let admin_id = create_admin(&state.pool).await;
    let admin_token = make_token(
        &admin_id.1,
        admin_id.0,
        raisfast::models::user::UserRole::Admin,
    );
    let (reader_tok, _) =
        register_and_login(&mut app, "reader@test.com", "reader", "Password123").await;

    let (s, _): (StatusCode, Value) = send(&mut app, get_auth("/api/v1/users", &reader_tok)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (status, body): (StatusCode, Value) =
        send(&mut app, get_auth("/api/v1/users", &admin_token)).await;
    assert!(status.is_success());
    assert!(body["data"]["items"].is_array());
    assert!(body["data"]["total"].is_number());
}

#[tokio::test]
async fn update_me_with_bio_and_website() {
    let (mut app, _) = test_app().await;
    let (access, _) =
        register_and_login(&mut app, "bioweb@test.com", "biowebuser", "Password123").await;
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        put_json_auth(
            "/api/v1/users/me",
            json!({"bio": "I write Rust", "website": "https://rust-lang.org"}),
            &access,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["bio"], "I write Rust");
    assert_eq!(body["data"]["website"], "https://rust-lang.org");
}

#[tokio::test]
async fn change_password_too_short() {
    let (mut app, _) = test_app().await;
    let (access, _) =
        register_and_login(&mut app, "short@test.com", "shortuser", "Password123").await;
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        put_json_auth(
            "/api/v1/users/me/password",
            json!({"old_password": "Password123", "new_password": "abc"}),
            &access,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{status} {body:?}");
}

#[tokio::test]
async fn admin_can_update_role() {
    let (mut app, state) = test_app().await;
    let admin_id = create_admin(&state.pool).await;
    let admin_token = make_token(
        &admin_id.1,
        admin_id.0,
        raisfast::models::user::UserRole::Admin,
    );
    let _ = register_and_login(&mut app, "roleuser@test.com", "roleuser", "Password123").await;
    let reader_id_i64: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'roleuser'")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    let reader_id = reader_id_i64.to_string();
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/users/{reader_id}/role"),
            json!({"role": "author"}),
            &admin_token,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["role"], "author");
}

#[tokio::test]
async fn get_user_by_id_returns_public_info() {
    let (mut app, state) = test_app().await;
    let (token, _) =
        register_and_login(&mut app, "pubinfo@test.com", "pubinfouser", "Password123").await;
    let user_id_i64: i64 =
        sqlx::query_scalar("SELECT id FROM users WHERE username = 'pubinfouser'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    let user_id = user_id_i64.to_string();
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        get_auth(&format!("/api/v1/users/{user_id}"), &token),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["id"], user_id);
    assert_eq!(body["data"]["username"], "pubinfouser");
    assert!(body["data"]["created_at"].is_string());
    assert!(body["data"]["updated_at"].is_string());
}

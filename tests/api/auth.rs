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
    assert_eq!(body["data"]["username"], "reguser");
    assert_eq!(body["data"]["role"], "reader");
}

#[tokio::test]
async fn register_duplicate_email() {
    let (mut app, _) = test_app().await;
    let req_body = json!({"email": "dup@test.com", "username": "dup1", "password": "Password123"});
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
    let (access, _) = register_and_login(&mut app, "lo@test.com", "louser", "Password123").await;
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

#[tokio::test]
async fn register_then_login_and_access_me() {
    let (mut app, _) = test_app().await;
    let (access, _) = register_and_login(
        &mut app,
        "lifecycle@test.com",
        "lifecycleuser",
        "Password123",
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/users/me", &access)).await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["username"], "lifecycleuser");
}

#[tokio::test]
async fn register_duplicate_username() {
    let (mut app, _) = test_app().await;
    let req_body =
        json!({"email": "dupu1@test.com", "username": "dupuser", "password": "Password123"});
    let (s, _): (StatusCode, Value) =
        send(&mut app, post_json("/api/v1/auth/register", req_body)).await;
    assert!(s.is_success());

    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            "/api/v1/auth/register",
            json!({"email": "dupu2@test.com", "username": "dupuser", "password": "Password123"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], 40900);
}

#[tokio::test]
async fn refresh_token_after_password_change() {
    let (mut app, _) = test_app().await;
    let (access, refresh) =
        register_and_login(&mut app, "rpc@test.com", "rpcuser", "OldPass123").await;

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

    let (status, _): (StatusCode, Value) = send(
        &mut app,
        post_json("/api/v1/auth/refresh", json!({"refresh_token": refresh})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "old refresh token should be invalidated after password change"
    );
}

#[tokio::test]
async fn logout_invalidates_access() {
    let (mut app, _) = test_app().await;
    let (access, _) =
        register_and_login(&mut app, "loinv@test.com", "loinvuser", "Password123").await;

    let (status, _): (StatusCode, Value) = send(
        &mut app,
        post_json_auth("/api/v1/auth/logout", json!({}), &access),
    )
    .await;
    assert!(status.is_success());

    let (status, _): (StatusCode, Value) =
        send(&mut app, get_auth("/api/v1/users/me", &access)).await;
    assert!(
        status.is_success(),
        "JWT is stateless — still valid after logout"
    );
}

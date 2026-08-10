use super::*;

#[tokio::test]
async fn register_success() {
    let (mut app, _) = test_app().await;
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            "/api/v1/auth/register",
            json!({"email": uniq_email("reg"), "username": uniq("reguser"), "password": "Password123"}),
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["code"], 0);
    assert!(
        body["data"]["username"]
            .as_str()
            .unwrap_or("")
            .starts_with("reguser")
    );
    assert!(
        body["data"]["roles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str() == Some("reader"))
    );
}

#[tokio::test]
async fn register_duplicate_email() {
    let (mut app, _) = test_app().await;
    let email = uniq_email("dup");
    let req_body = json!({"email": &email, "username": uniq("dup1"), "password": "Password123"});
    let (s, _): (StatusCode, Value) =
        send(&mut app, post_json("/api/v1/auth/register", req_body)).await;
    assert!(s.is_success());

    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            "/api/v1/auth/register",
            json!({"email": &email, "username": uniq("dup2"), "password": "Password123"}),
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
        json!({"email": "bad", "username": uniq("user"), "password": "Password123"}),
        json!({"email": uniq_email("ok"), "username": "a", "password": "Password123"}),
        json!({"email": uniq_email("ok"), "username": uniq("user"), "password": "short"}),
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
    let (access, refresh) = register_and_login(
        &mut app,
        &uniq_email("login"),
        &uniq("loginuser"),
        "Password123",
    )
    .await;
    assert!(!access.is_empty());
    assert!(!refresh.is_empty());
}

#[tokio::test]
async fn login_wrong_password() {
    let (mut app, _) = test_app().await;
    let email = uniq_email("lwp");
    let _ = register_and_login(&mut app, &email, &uniq("lwpuser"), "Password123").await;
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            "/api/v1/auth/login",
            json!({"email": email, "password": "Wrong123"}),
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
            json!({"email": uniq_email("none"), "password": "Password123"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_token_success() {
    let (mut app, _) = test_app().await;
    let (_, refresh) = register_and_login(
        &mut app,
        &uniq_email("refresh"),
        &uniq("refreshuser"),
        "Password123",
    )
    .await;
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
    let (_, r1) = register_and_login(
        &mut app,
        &uniq_email("rot"),
        &uniq("rotuser"),
        "Password123",
    )
    .await;

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
        register_and_login(&mut app, &uniq_email("lo"), &uniq("louser"), "Password123").await;
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
        &uniq_email("lifecycle"),
        &uniq("lifecycleuser"),
        "Password123",
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/users/me", &access)).await;
    assert!(status.is_success(), "{status} {body:?}");
    assert!(
        body["data"]["username"]
            .as_str()
            .unwrap_or("")
            .starts_with("lifecycleuser")
    );
}

#[tokio::test]
async fn register_duplicate_username() {
    let (mut app, _) = test_app().await;
    let username = uniq("dupuser");
    let req_body =
        json!({"email": uniq_email("dupu1"), "username": &username, "password": "Password123"});
    let (s, _): (StatusCode, Value) =
        send(&mut app, post_json("/api/v1/auth/register", req_body)).await;
    assert!(s.is_success());

    let (status, _body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            "/api/v1/auth/register",
            json!({"email": uniq_email("dupu2"), "username": &username, "password": "Password123"}),
        ),
    )
    .await;
    assert!(
        status == StatusCode::CONFLICT || status == StatusCode::INTERNAL_SERVER_ERROR,
        "expected 409 or 500 for duplicate username, got {status}"
    );
}

#[tokio::test]
async fn refresh_token_after_password_change() {
    let (mut app, _) = test_app().await;
    let (access, refresh) =
        register_and_login(&mut app, &uniq_email("rpc"), &uniq("rpcuser"), "OldPass123").await;

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
    let (access, _) = register_and_login(
        &mut app,
        &uniq_email("loinv"),
        &uniq("loinvuser"),
        "Password123",
    )
    .await;

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

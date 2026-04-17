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

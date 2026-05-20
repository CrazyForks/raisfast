use super::*;

async fn setup() -> (axum::Router, AppState, String) {
    let (mut app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
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
    let id = b["data"]["id"].as_str().unwrap().to_string();
    let (status, _): (StatusCode, Value) =
        send(&mut app, delete_auth(&format!("/api/v1/tags/{id}"), &tok)).await;
    assert!(status.is_success());
}

#[tokio::test]
async fn delete_not_found() {
    let (mut app, _, tok) = setup().await;
    let fake = "9999999999999";
    let (status, _): (StatusCode, Value) =
        send(&mut app, delete_auth(&format!("/api/v1/tags/{fake}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

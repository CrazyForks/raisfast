use super::*;

async fn setup() -> (axum::Router, AppState, String) {
    let (mut app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    let _: (StatusCode, Value) = send(&mut app, get_req("/api/v1/categories")).await;
    (app, state, tok)
}

#[tokio::test]
async fn list_empty() {
    let (mut app, _, _) = setup().await;
    let (status, body): (StatusCode, Value) = send(&mut app, get_req("/api/v1/categories")).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
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

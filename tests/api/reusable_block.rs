use super::*;

async fn admin_setup() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    (app, state, tok)
}

fn block_content() -> serde_json::Value {
    json!([{"type": "richtext", "content": "Hello world"}])
}

#[tokio::test]
async fn create_reusable_block_success() {
    let (mut app, _, tok) = admin_setup().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/reusable-blocks",
            json!({
                "name": "Hero Banner",
                "block_type": "hero",
                "content": block_content().to_string()
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create block: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Hero Banner");
    assert!(body["data"]["id"].is_string());
}

#[tokio::test]
async fn create_block_validation_empty_name() {
    let (mut app, _, tok) = admin_setup().await;
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/reusable-blocks",
            json!({"name": "", "block_type": "hero", "content": "[]"}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn list_reusable_blocks() {
    let (mut app, _, tok) = admin_setup().await;
    let _: (StatusCode, Value) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/reusable-blocks",
            json!({"name": "Block1", "block_type": "text", "content": "[]"}),
            &tok,
        ),
    )
    .await;
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/reusable-blocks", &tok)).await;
    assert!(status.is_success(), "list blocks: {status} {body:?}");
    let items = body["data"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn get_reusable_block_by_id() {
    let (mut app, _, tok) = admin_setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/reusable-blocks",
            json!({"name": "CTA", "block_type": "cta", "content": "[]"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();
    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/reusable-blocks/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "get block: {status} {body:?}");
    assert_eq!(body["data"]["name"], "CTA");
}

#[tokio::test]
async fn get_reusable_block_not_found() {
    let (mut app, _, tok) = admin_setup().await;
    let fake = "9999999999999";
    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/reusable-blocks/{fake}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_reusable_block() {
    let (mut app, _, tok) = admin_setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/reusable-blocks",
            json!({"name": "Old", "block_type": "text", "content": "[]"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/reusable-blocks/{id}"),
            json!({"name": "New", "description": "updated"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update block: {status} {body:?}");
    assert_eq!(body["data"]["name"], "New");
}

#[tokio::test]
async fn delete_reusable_block() {
    let (mut app, _, tok) = admin_setup().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/reusable-blocks",
            json!({"name": "Delete", "block_type": "text", "content": "[]"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();
    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/reusable-blocks/{id}"), &tok),
    )
    .await;
    assert!(status.is_success());
    let (s, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/reusable-blocks/{id}"), &tok),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_reusable_block_not_found() {
    let (mut app, _, tok) = admin_setup().await;
    let fake = "9999999999999";
    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/reusable-blocks/{fake}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

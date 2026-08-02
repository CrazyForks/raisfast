use super::*;

async fn admin_author() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    (app, state, tok)
}

#[tokio::test]
async fn create_page_success() {
    let (mut app, _, tok) = admin_author().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "About Us", "content": "Our story", "status": "published"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create page: {status} {body:?}");
    assert_eq!(body["data"]["title"], "About Us");
    assert!(body["data"]["slug"].is_string());
}

#[tokio::test]
async fn create_page_validation_empty_title() {
    let (mut app, _, tok) = admin_author().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "", "content": "c", "status": "published"}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success(), "expected failure: {status} {body:?}");
}

#[tokio::test]
async fn list_published_pages() {
    let (mut app, _, tok) = admin_author().await;
    let _: (StatusCode, Value) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "Page1", "content": "c", "status": "published"}),
            &tok,
        ),
    )
    .await;
    let (status, body) = send(&mut app, get_req("/api/v1/pages")).await;
    assert!(status.is_success(), "list pages: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn get_page_by_slug() {
    let (mut app, _, tok) = admin_author().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "Contact", "content": "info", "status": "published"}),
            &tok,
        ),
    )
    .await;
    let slug = create_body["data"]["slug"].as_str().unwrap();
    let (status, body) = send(&mut app, get_req(&format!("/api/v1/pages/{slug}"))).await;
    assert!(status.is_success(), "get page: {status} {body:?}");
    assert_eq!(body["data"]["title"], "Contact");
}

#[tokio::test]
async fn get_page_not_found() {
    let (mut app, _, _) = admin_author().await;
    let (status, _) = send(&mut app, get_req("/api/v1/pages/nonexistent")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_page() {
    let (mut app, _, tok) = admin_author().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "Original", "content": "c", "status": "published"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/pages/{id}"),
            json!({"title": "Updated"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update page: {status} {body:?}");
    assert_eq!(body["data"]["title"], "Updated");
}

#[tokio::test]
async fn delete_page() {
    let (mut app, _, tok) = admin_author().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "ToDelete", "content": "c", "status": "published"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();
    let slug = create_body["data"]["slug"].as_str().unwrap();
    let (status, _) = send(&mut app, delete_auth(&format!("/api/v1/pages/{id}"), &tok)).await;
    assert!(status.is_success());
    let (s, _) = send(&mut app, get_req(&format!("/api/v1/pages/{slug}"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_list_pages() {
    let (mut app, _, tok) = admin_author().await;
    let _: (StatusCode, Value) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "Admin Page", "content": "c", "status": "draft"}),
            &tok,
        ),
    )
    .await;
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/pages", &tok)).await;
    assert!(status.is_success(), "admin list: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn update_page_status() {
    let (mut app, _state, tok) = admin_author().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/pages",
            json!({"title": "StatusTest", "content": "c", "status": "draft"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/pages/{id}/status"),
            json!({"status": "published"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update status: {status} {body:?}");
}

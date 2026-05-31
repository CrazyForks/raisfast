use super::*;

async fn setup_admin() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);
    (app, state, tok)
}

async fn setup_author() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    (app, state, tok)
}

#[tokio::test]
async fn admin_create_product_category() {
    let (mut app, _, tok) = setup_admin().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Electronics", "description": "All electronic items"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Electronics");
    assert_eq!(body["data"]["slug"], "electronics");
    assert_eq!(body["data"]["description"], "All electronic items");
}

#[tokio::test]
async fn admin_create_product_category_validation() {
    let (mut app, _, tok) = setup_admin().await;
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": ""}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn admin_update_product_category() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Phones"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/product-categories/{id}"),
            json!({"name": "Smartphones", "description": "Mobile phones", "sort_order": 10}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Smartphones");
    assert_eq!(body["data"]["sort_order"], 10);
}

#[tokio::test]
async fn admin_delete_product_category() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Bye"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/product-categories/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete: {status}");
}

#[tokio::test]
async fn admin_list_product_categories() {
    let (mut app, _, tok) = setup_admin().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "A"}),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "B"}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/product-categories?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success());
    let items = body["data"]["items"].as_array().unwrap();
    assert!(items.len() >= 2);
}

#[tokio::test]
async fn public_list_product_categories() {
    let (mut app, _, tok) = setup_admin().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Public"}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_req("/api/v1/product-categories?page=1&page_size=10"),
    )
    .await;
    assert!(status.is_success());
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn get_product_category_by_id() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Found"}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        get_req(&format!("/api/v1/product-categories/{id}")),
    )
    .await;
    assert!(status.is_success(), "get: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Found");
}

#[tokio::test]
async fn create_with_parent_category() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, parent_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Parent"}),
            &tok,
        ),
    )
    .await;
    let parent_id = parent_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Child", "parent_id": parent_id}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create child: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Child");
}

#[tokio::test]
async fn author_can_create_and_update() {
    let (mut app, _, tok) = setup_author().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/product-categories",
            json!({"name": "AuthorCat"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "author create: {status} {body:?}");
    assert_eq!(body["data"]["name"], "AuthorCat");

    let id = body["data"]["id"].as_str().unwrap();
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/product-categories/{id}"),
            json!({"name": "Updated"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "author update: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Updated");
}

#[tokio::test]
async fn admin_batch_delete() {
    let (mut app, _, tok) = setup_admin().await;
    let mut ids = Vec::new();
    for i in 0..3 {
        let (_, b) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/product-categories",
                json!({"name": format!("Batch{i}")}),
                &tok,
            ),
        )
        .await;
        ids.push(b["data"]["id"].as_str().unwrap().to_string());
    }

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories/batch",
            json!({"action": "delete", "ids": ids}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "batch: {status} {body:?}");
    assert_eq!(body["data"]["affected"], 3);
}

use super::*;

async fn setup_admin() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);
    (app, state, tok)
}

#[tokio::test]
async fn admin_create_product() {
    let (mut app, _, tok) = setup_admin().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Widget", "price": 9900, "currency": "CNY"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create: {status} {body:?}");
    assert_eq!(body["data"]["title"], "Widget");
    assert_eq!(body["data"]["price"], 9900);
    assert_eq!(body["data"]["status"], "draft");
}

#[tokio::test]
async fn admin_create_product_validation() {
    let (mut app, _, tok) = setup_admin().await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/admin/products", json!({"price": 100}), &tok),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn admin_update_product() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Old", "price": 1000}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/products/{id}"),
            json!({"title": "New", "price": 2000, "status": "active", "version": 1}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update: {status} {body:?}");
    assert_eq!(body["data"]["title"], "New");
    assert_eq!(body["data"]["price"], 2000);
    assert_eq!(body["data"]["version"], 2);
}

#[tokio::test]
async fn admin_update_version_conflict() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Conflict", "price": 1000}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/products/{id}"),
            json!({"title": "X", "version": 999}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn admin_delete_product() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Bye", "price": 100}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/products/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete: {status}");
}

#[tokio::test]
async fn list_active_products_only() {
    let (mut app, state, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Active", "price": 100}),
            &tok,
        ),
    )
    .await;
    let pid = create_body["data"]["id"].as_str().unwrap();
    let int_id: i64 = sqlx::query_scalar("SELECT id FROM products WHERE id = ?")
        .bind(pid)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
        .bind(int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Draft", "price": 200}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(&mut app, get_req("/api/v1/products")).await;
    assert!(status.is_success());
    let items = body["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Active");
}

#[tokio::test]
async fn get_product_by_id() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Found", "price": 500}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/products/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "get: {status} {body:?}");
    assert_eq!(body["data"]["title"], "Found");
}

#[tokio::test]
async fn admin_list_products() {
    let (mut app, _, tok) = setup_admin().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "P1", "price": 100}),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "P2", "price": 200}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/products?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success());
    let items = body["data"]["items"].as_array().unwrap();
    assert!(items.len() >= 2);
}

#[tokio::test]
async fn admin_create_product_with_category() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, cat_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Electronics"}),
            &tok,
        ),
    )
    .await;
    let cat_id = cat_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Phone", "price": 4999, "currency": "CNY", "category_id": cat_id}),
            &tok,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "create with category: {status} {body:?}"
    );
    assert_eq!(body["data"]["title"], "Phone");
    assert_eq!(body["data"]["category_id"], cat_id);
}

#[tokio::test]
async fn admin_update_product_category() {
    let (mut app, _, tok) = setup_admin().await;
    let (_, cat_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-categories",
            json!({"name": "Tablets"}),
            &tok,
        ),
    )
    .await;
    let cat_id = cat_body["data"]["id"].as_str().unwrap();

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "iPad", "price": 3000}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/products/{id}"),
            json!({"title": "iPad Pro", "price": 6000, "version": 1, "category_id": cat_id}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update category: {status} {body:?}");
    assert_eq!(body["data"]["category_id"], cat_id);
}

#[tokio::test]
async fn admin_create_product_with_invalid_category() {
    let (mut app, _, tok) = setup_admin().await;
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Ghost", "price": 100, "category_id": "99999"}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

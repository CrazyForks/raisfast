use super::*;

async fn setup_with_product() -> (axum::Router, AppState, String, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (_, pbody) = send(
        &mut app.clone(),
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Cart Product", "price": 5000, "currency": "CNY"}),
            &tok,
        ),
    )
    .await;
    let product_id = pbody["data"]["id"].as_str().unwrap().to_string();
    let int_id: i64 = sqlx::query_scalar("SELECT id FROM products WHERE id = ?")
        .bind(&product_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
        .bind(int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    (app, state, tok, product_id)
}

#[tokio::test]
async fn add_to_cart() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/cart",
            json!({"product_id": product_id, "quantity": 2}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "add to cart: {status}");
}

#[tokio::test]
async fn add_to_cart_merges_quantity() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/cart",
            json!({"product_id": product_id, "quantity": 2}),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/cart",
            json!({"product_id": product_id, "quantity": 3}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    assert!(status.is_success(), "list cart: {status} {body:?}");
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"]["items"][0]["quantity"], 5);
}

#[tokio::test]
async fn list_cart_empty() {
    let (mut app, _, tok, _) = setup_with_product().await;
    let (status, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["data"]["total"], 0);
}

#[tokio::test]
async fn update_cart_quantity() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    let (_, _add_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/cart",
            json!({"product_id": product_id, "quantity": 1}),
            &tok,
        ),
    )
    .await;

    let (_, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    let cart_item_id = body["data"]["items"][0]["id"].as_str().unwrap();

    let (_status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/cart/{cart_item_id}"),
            json!({"quantity": 10}),
            &tok,
        ),
    )
    .await;

    let (_, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    assert_eq!(body["data"]["items"][0]["quantity"], 10);
}

#[tokio::test]
async fn remove_from_cart() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/cart",
            json!({"product_id": product_id, "quantity": 1}),
            &tok,
        ),
    )
    .await;

    let (_, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    let cart_item_id = body["data"]["items"][0]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/cart/{cart_item_id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "remove: {status}");

    let (_, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn clear_cart() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/cart",
            json!({"product_id": product_id, "quantity": 1}),
            &tok,
        ),
    )
    .await;

    let (status, _) = send(&mut app, delete_auth("/api/v1/cart", &tok)).await;
    assert!(status.is_success(), "clear: {status}");

    let (_, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn checkout_creates_order() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/cart",
            json!({"product_id": product_id, "quantity": 2}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/cart/checkout", json!({}), &tok),
    )
    .await;
    assert!(status.is_success(), "checkout: {status} {body:?}");
    assert_eq!(body["data"]["total_amount"], 10000);
    assert_eq!(body["data"]["status"], "pending");
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);

    let (_, body) = send(&mut app, get_auth("/api/v1/cart", &tok)).await;
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn checkout_empty_cart_fails() {
    let (mut app, _, tok, _) = setup_with_product().await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/cart/checkout", json!({}), &tok),
    )
    .await;
    assert!(!status.is_success());
}

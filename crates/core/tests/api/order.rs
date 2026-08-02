use super::*;

async fn setup_with_product() -> (axum::Router, AppState, String, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (_, pbody) = send(
        &mut app.clone(),
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Test Product", "price": 9900, "currency": "CNY", "stock": 100}),
            &tok,
        ),
    )
    .await;
    let product_id = pbody["data"]["id"].as_str().unwrap().to_string();
    let product_int_id: i64 = sqlx::query_scalar("SELECT id FROM products WHERE id = ?")
        .bind(&product_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
        .bind(product_int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    (app, state, tok, product_id)
}

#[tokio::test]
async fn create_order_success() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 2}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create order: {status} {body:?}");
    assert_eq!(body["data"]["total_amount"], 19800);
    assert_eq!(body["data"]["status"], "pending");
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn create_order_validation_empty_items() {
    let (mut app, _, tok, _) = setup_with_product().await;

    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({"items": [], "currency": "CNY"}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn get_order_by_id() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;
    let order_id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/orders/{order_id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "get order: {status} {body:?}");
    assert_eq!(body["data"]["id"], order_id);
}

#[tokio::test]
async fn list_user_orders() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/orders?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success(), "list orders: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn cancel_own_order() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;
    let order_id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        put_json_auth(&format!("/api/v1/orders/{order_id}"), json!({}), &tok),
    )
    .await;
    assert!(status.is_success(), "cancel order: {status}");

    let (_, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/orders/{order_id}"), &tok),
    )
    .await;
    assert_eq!(body["data"]["status"], "cancelled");
}

#[tokio::test]
async fn admin_list_orders() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/orders?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success(), "admin list orders: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn admin_ship_order() {
    let (mut app, state, tok, product_id) = setup_with_product().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;
    let order_id = create_body["data"]["id"].as_str().unwrap();

    let order_int_id: i64 = sqlx::query_scalar("SELECT id FROM orders WHERE id = ?")
        .bind(order_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE orders SET status = 'paid' WHERE id = ?")
        .bind(order_int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    let (status, _) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/orders/{order_id}/ship"),
            json!({"tracking_no": "SF1234567890", "carrier": "SF Express"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "ship: {status}");

    let (_, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/orders/{order_id}"), &tok),
    )
    .await;
    assert_eq!(body["data"]["status"], "shipped");
    assert_eq!(body["data"]["tracking_no"], "SF1234567890");
}

#[tokio::test]
async fn admin_cancel_order() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;
    let order_id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/orders/{order_id}/cancel"),
            json!({}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "admin cancel: {status}");
}

#[tokio::test]
async fn admin_update_remark() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;
    let order_id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/orders/{order_id}/remark"),
            json!({"admin_remark": "VIP customer"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update remark: {status}");

    let (_, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/orders/{order_id}"), &tok),
    )
    .await;
    assert_eq!(body["data"]["admin_remark"], "VIP customer");
}

#[tokio::test]
async fn admin_stats() {
    let (mut app, _, tok, product_id) = setup_with_product().await;

    let _ = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({
                "items": [{"product_id": product_id, "quantity": 1}],
                "currency": "CNY",
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/admin/orders/stats", &tok)).await;
    assert!(status.is_success(), "stats: {status} {body:?}");
    assert!(body["data"]["total_orders"].as_i64().unwrap() >= 1);
}

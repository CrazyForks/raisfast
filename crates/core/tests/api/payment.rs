use super::*;

async fn setup_admin_with_channel() -> (axum::Router, AppState, String, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (_, cbody) = send(
        &mut app.clone(),
        post_json_auth(
            "/api/v1/admin/payment/channels",
            json!({
                "provider": "stripe",
                "name": "Test Channel",
                "credentials": "{\"api_key\":\"sk_test_123\"}",
                "is_live": false,
            }),
            &tok,
        ),
    )
    .await;
    let channel_id = cbody["data"]["id"].as_str().unwrap().to_string();

    (app, state, tok, channel_id)
}

async fn setup_admin_with_routing_channels() -> (axum::Router, AppState, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    send(
        &mut app.clone(),
        post_json_auth(
            "/api/v1/admin/payment/channels",
            json!({
                "provider": "stripe",
                "name": "Stripe CN",
                "credentials": "{\"api_key\":\"test\"}",
                "settings": "{\"countries\":[\"CN\"],\"currencies\":[\"CNY\"],\"priority\":100}",
                "is_live": false,
            }),
            &tok,
        ),
    )
    .await;

    send(
        &mut app.clone(),
        post_json_auth(
            "/api/v1/admin/payment/channels",
            json!({
                "provider": "stripe",
                "name": "Stripe Global",
                "credentials": "{\"api_key\":\"test\"}",
                "settings": "{\"countries\":[\"*\"],\"currencies\":[\"USD\",\"CNY\"],\"priority\":10}",
                "is_live": false,
            }),
            &tok,
        ),
    )
    .await;

    (app, state, tok)
}

#[tokio::test]
async fn admin_create_channel() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/payment/channels",
            json!({
                "provider": "stripe",
                "name": "Stripe Test",
                "credentials": "{\"api_key\":\"sk_test_123\"}",
                "is_live": false,
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create channel: {status} {body:?}");
    assert_eq!(body["data"]["provider"], "stripe");
    assert_eq!(body["data"]["name"], "Stripe Test");
    assert!(!body["data"]["is_live"].as_bool().unwrap());
    assert_eq!(body["data"]["credentials_masked"], "[encrypted]");
}

#[tokio::test]
async fn admin_create_channel_validation() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/payment/channels",
            json!({"provider": "", "name": "x", "credentials": ""}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn admin_list_channels() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/payment/channels?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success(), "list channels: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn admin_get_channel() {
    let (mut app, _, tok, channel_id) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/payment/channels/{channel_id}"),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "get channel: {status} {body:?}");
    assert_eq!(body["data"]["id"], channel_id);
}

#[tokio::test]
async fn admin_update_channel() {
    let (mut app, _, tok, channel_id) = setup_admin_with_channel().await;

    let (_, get_body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/payment/channels/{channel_id}"),
            &tok,
        ),
    )
    .await;
    let version = get_body["data"]["version"].as_i64().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/payment/channels/{channel_id}"),
            json!({"name": "Updated Channel", "version": version}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update channel: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Updated Channel");
}

#[tokio::test]
async fn admin_delete_channel() {
    let (mut app, _, tok, channel_id) = setup_admin_with_channel().await;

    let (status, _) = send(
        &mut app,
        delete_auth(
            &format!("/api/v1/admin/payment/channels/{channel_id}"),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "delete channel");

    let (status, _) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/payment/channels/{channel_id}"),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_payment_order_requires_auth() {
    let (mut app, _, _, _) = setup_admin_with_channel().await;

    let (status, _) = send(
        &mut app,
        post_json(
            "/api/v1/payment/orders",
            json!({"order_id": "fake", "channel_id": "fake"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_user_payment_orders_empty() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/payment/orders?page=1&page_size=10", &tok),
    )
    .await;
    assert!(
        status.is_success(),
        "list payment orders: {status} {body:?}"
    );
}

#[tokio::test]
async fn admin_list_payment_orders() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/payment/orders?page=1&page_size=10", &tok),
    )
    .await;
    assert!(
        status.is_success(),
        "admin list payment orders: {status} {body:?}"
    );
}

#[tokio::test]
async fn admin_list_transactions_empty() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth(
            "/api/v1/admin/payment/transactions?page=1&page_size=10",
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "list txns: {status} {body:?}");
}

#[tokio::test]
async fn admin_list_refunds_empty() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/payment/refunds?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success(), "list refunds: {status} {body:?}");
}

#[tokio::test]
async fn list_available_channels_returns_channels() {
    let (mut app, state, tok) = setup_admin_with_routing_channels().await;

    let product_id: String;
    {
        let (_, pbody) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/products",
                json!({"title": "Pay Product", "price": 9900, "currency": "CNY", "stock": 100}),
                &tok,
            ),
        )
        .await;
        product_id = pbody["data"]["id"].as_str().unwrap().to_string();
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
    }

    let (_, obody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({"items": [{"product_id": product_id, "quantity": 1}], "currency": "CNY"}),
            &tok,
        ),
    )
    .await;
    let order_id = obody["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        get_auth(
            &format!(
                "/api/v1/payment/channels/available?order_id={order_id}&country=CN&language=zh"
            ),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "available channels: {status} {body:?}");

    let channels = body["data"]["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
    assert!(channels[0]["is_recommended"].as_bool().unwrap());
    assert_eq!(channels[0]["provider"], "stripe");
    assert_eq!(
        body["data"]["recommended_channel_id"],
        channels[0]["channel_id"]
    );
}

#[tokio::test]
async fn list_available_channels_no_match() {
    let (mut app, state, tok) = setup_admin_with_routing_channels().await;

    let (_, pbody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "JPY Product", "price": 1000, "currency": "JPY", "stock": 100}),
            &tok,
        ),
    )
    .await;
    let product_id = pbody["data"]["id"].as_str().unwrap();
    let product_int_id: i64 = sqlx::query_scalar("SELECT id FROM products WHERE id = ?")
        .bind(product_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
        .bind(product_int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    let (_, obody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({"items": [{"product_id": product_id, "quantity": 1}], "currency": "JPY"}),
            &tok,
        ),
    )
    .await;
    let order_id = obody["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/payment/channels/available?order_id={order_id}"),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "available: {status} {body:?}");
    let channels = body["data"]["channels"].as_array().unwrap();
    assert!(channels.is_empty());
    assert!(body["data"]["recommended_channel_id"].is_null());
}

#[tokio::test]
async fn list_available_channels_requires_auth() {
    let (mut app, _, _) = setup_admin_with_routing_channels().await;

    let (status, _) = send(
        &mut app,
        get_req("/api/v1/payment/channels/available?order_id=fake"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[cfg(feature = "payment-stripe")]
async fn create_payment_order_auto_route() {
    let (mut app, state, tok) = setup_admin_with_routing_channels().await;

    let (_, pbody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Auto Product", "price": 9900, "currency": "CNY", "stock": 100}),
            &tok,
        ),
    )
    .await;
    let product_id = pbody["data"]["id"].as_str().unwrap();
    let product_int_id: i64 = sqlx::query_scalar("SELECT id FROM products WHERE id = ?")
        .bind(product_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
        .bind(product_int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    let (_, obody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({"items": [{"product_id": product_id, "quantity": 1}], "currency": "CNY"}),
            &tok,
        ),
    )
    .await;
    let order_id = obody["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("POST")
            .uri("/api/v1/payment/orders")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {tok}"))
            .header("Accept-Language", "zh-CN")
            .header("User-Agent", "TestAgent/1.0")
            .body(Body::from(
                serde_json::to_string(&json!({
                    "order_id": order_id,
                    "country": "CN",
                    "language": "zh-CN",
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await;

    assert!(status.is_success(), "auto route create: {status} {body:?}");
    assert_eq!(
        body["data"]["provider"].as_str().unwrap_or("none"),
        "stripe"
    );
    assert_eq!(
        body["data"]["channel_selected_by"]
            .as_str()
            .unwrap_or("none"),
        "auto"
    );
    assert_eq!(
        body["data"]["client_country"].as_str().unwrap_or("none"),
        "CN"
    );
}

#[tokio::test]
#[cfg(feature = "payment-stripe")]
async fn create_payment_order_manual_channel() {
    let (mut app, state, tok, channel_id) = setup_admin_with_channel().await;

    let (_, pbody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Pay Product", "price": 9900, "currency": "CNY"}),
            &tok,
        ),
    )
    .await;
    let product_id = pbody["data"]["id"].as_str().unwrap();
    let product_int_id: i64 = sqlx::query_scalar("SELECT id FROM products WHERE id = ?")
        .bind(product_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
        .bind(product_int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    let (status, obody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({"items": [{"product_id": product_id, "quantity": 1}], "currency": "CNY"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create order: {status} {obody:?}");
    let order_id = obody["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/payment/orders",
            json!({
                "order_id": order_id,
                "channel_id": channel_id,
                "country": "US",
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "manual create: {status} {body:?}");
    assert_eq!(body["data"]["channel_selected_by"], "manual");
    assert_eq!(body["data"]["client_country"], "US");
}

#[tokio::test]
async fn create_payment_order_no_channel_no_match() {
    let (mut app, state, tok) = setup_admin_with_routing_channels().await;

    let (_, pbody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "JPY Product", "price": 5000, "currency": "JPY", "stock": 100}),
            &tok,
        ),
    )
    .await;
    let product_id = pbody["data"]["id"].as_str().unwrap();
    let product_int_id: i64 = sqlx::query_scalar("SELECT id FROM products WHERE id = ?")
        .bind(product_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
        .bind(product_int_id)
        .execute(&state.pool)
        .await
        .unwrap();

    let (_, obody) = send(
        &mut app,
        post_json_auth(
            "/api/v1/orders",
            json!({"items": [{"product_id": product_id, "quantity": 1}], "currency": "JPY"}),
            &tok,
        ),
    )
    .await;
    let order_id = obody["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/payment/orders",
            json!({"order_id": order_id}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success(), "should fail: {status} {body:?}");
}

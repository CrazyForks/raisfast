use super::*;

async fn setup_admin_with_channel() -> (axum::Router, AppState, String, String) {
    let (app, state) = test_app().await;
    let (int_id, doc_id) = create_admin(&state.pool).await;
    let tok = make_token(&doc_id, int_id, raisfast::models::user::UserRole::Admin);

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
        get_auth(&format!("/api/v1/admin/payment/channels/{channel_id}"), &tok),
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
        get_auth(&format!("/api/v1/admin/payment/channels/{channel_id}"), &tok),
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
        delete_auth(&format!("/api/v1/admin/payment/channels/{channel_id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete channel");

    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/payment/channels/{channel_id}"), &tok),
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
    assert!(status.is_success(), "list payment orders: {status} {body:?}");
}

#[tokio::test]
async fn admin_list_payment_orders() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/payment/orders?page=1&page_size=10", &tok),
    )
    .await;
    assert!(status.is_success(), "admin list payment orders: {status} {body:?}");
}

#[tokio::test]
async fn admin_list_transactions_empty() {
    let (mut app, _, tok, _) = setup_admin_with_channel().await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/payment/transactions?page=1&page_size=10", &tok),
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

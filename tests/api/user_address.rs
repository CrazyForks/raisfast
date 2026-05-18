use super::*;

async fn setup_user() -> (axum::Router, AppState, String) {
    let (mut app, state) = test_app().await;
    let (access_token, _) =
        register_and_login(&mut app, "addr_user@test.com", "addruser", "Password123!").await;
    (app, state, access_token)
}

fn sample_address() -> Value {
    json!({
        "recipient_name": "Zhang San",
        "phone": "13800138000",
        "country": "CN",
        "province": "Guangdong",
        "city": "Shenzhen",
        "district": "Nanshan",
        "address_line1": "123 Tech Park Road",
        "postal_code": "518000",
        "is_default": true,
        "address_type": "shipping",
    })
}

#[tokio::test]
async fn create_address() {
    let (mut app, _, tok) = setup_user().await;
    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/user/addresses", sample_address(), &tok),
    )
    .await;
    assert!(status.is_success(), "create address: {status} {body:?}");
    assert_eq!(body["data"]["recipient_name"], "Zhang San");
    assert_eq!(body["data"]["phone"], "13800138000");
    assert_eq!(body["data"]["province"], "Guangdong");
    assert!(body["data"]["is_default"].as_bool().unwrap());
}

#[tokio::test]
async fn create_address_validation() {
    let (mut app, _, tok) = setup_user().await;
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/user/addresses", json!({}), &tok),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn list_addresses() {
    let (mut app, _, tok) = setup_user().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/user/addresses",
            json!({
                "recipient_name": "A",
                "phone": "111",
                "address_line1": "Addr A",
            }),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/user/addresses",
            json!({
                "recipient_name": "B",
                "phone": "222",
                "address_line1": "Addr B",
            }),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/user/addresses", &tok)).await;
    assert!(status.is_success(), "list addresses: {status} {body:?}");
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn list_addresses_empty() {
    let (mut app, _, tok) = setup_user().await;
    let (status, body) = send(&mut app, get_auth("/api/v1/user/addresses", &tok)).await;
    assert!(status.is_success());
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn update_address() {
    let (mut app, _, tok) = setup_user().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth("/api/v1/user/addresses", sample_address(), &tok),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/user/addresses/{id}"),
            json!({"recipient_name": "Li Si", "phone": "13900139000", "city": "Beijing"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update address: {status} {body:?}");
    assert_eq!(body["data"]["recipient_name"], "Li Si");
    assert_eq!(body["data"]["phone"], "13900139000");
    assert_eq!(body["data"]["city"], "Beijing");
}

#[tokio::test]
async fn delete_address() {
    let (mut app, _, tok) = setup_user().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth("/api/v1/user/addresses", sample_address(), &tok),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/user/addresses/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete address: {status}");

    let (_, body) = send(&mut app, get_auth("/api/v1/user/addresses", &tok)).await;
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn cannot_operate_other_users_address() {
    let (mut app, _, tok) = setup_user().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth("/api/v1/user/addresses", sample_address(), &tok),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (access_token2, _) =
        register_and_login(&mut app, "addr_user2@test.com", "addruser2", "Password123!").await;

    let (status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/user/addresses/{id}"),
            json!({"recipient_name": "Hacked"}),
            &access_token2,
        ),
    )
    .await;
    assert!(!status.is_success(), "should not update other user address");

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/user/addresses/{id}"), &access_token2),
    )
    .await;
    assert!(!status.is_success(), "should not delete other user address");
}

#[tokio::test]
async fn address_type_shipping_and_billing() {
    let (mut app, _, tok) = setup_user().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/user/addresses",
            json!({
                "recipient_name": "Office",
                "phone": "111",
                "address_line1": "456 Business Ave",
                "address_type": "billing",
            }),
            &tok,
        ),
    )
    .await;
    assert_eq!(create_body["data"]["address_type"], "billing");
}

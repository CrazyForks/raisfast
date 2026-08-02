use super::*;

async fn setup_with_product() -> (axum::Router, AppState, String, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (_, pbody) = send(
        &mut app.clone(),
        post_json_auth(
            "/api/v1/admin/products",
            json!({"title": "Variant Product", "price": 5000}),
            &tok,
        ),
    )
    .await;
    let product_id = pbody["data"]["id"].as_str().unwrap().to_string();
    (app, state, tok, product_id)
}

#[tokio::test]
async fn admin_create_variant() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({
                "product_id": product_id,
                "title": "Red / M",
                "sku": "SKU-RED-M",
                "price": 6000,
                "stock": 50,
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create variant: {status} {body:?}");
    assert_eq!(body["data"]["title"], "Red / M");
    assert_eq!(body["data"]["sku"], "SKU-RED-M");
    assert_eq!(body["data"]["price"], 6000);
    assert_eq!(body["data"]["stock"], 50);
}

#[tokio::test]
async fn admin_create_variant_validation() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({"product_id": product_id}),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn admin_update_variant() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({
                "product_id": product_id,
                "title": "Blue",
                "price": 5000,
            }),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/product-variants/{id}"),
            json!({"title": "Green", "price": 7000}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update variant: {status} {body:?}");
    assert_eq!(body["data"]["title"], "Green");
    assert_eq!(body["data"]["price"], 7000);
}

#[tokio::test]
async fn admin_delete_variant() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({
                "product_id": product_id,
                "title": "Bye",
                "price": 1000,
            }),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/product-variants/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete variant: {status}");
}

#[tokio::test]
async fn list_variants_by_product() {
    let (mut app, _, tok, product_id) = setup_with_product().await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({"product_id": product_id, "title": "V1", "price": 100}),
            &tok,
        ),
    )
    .await;
    send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({"product_id": product_id, "title": "V2", "price": 200}),
            &tok,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/products/{product_id}/variants"), &tok),
    )
    .await;
    assert!(status.is_success(), "list variants: {status} {body:?}");
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn create_variant_nonexistent_product() {
    let (mut app, _, tok, _) = setup_with_product().await;
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({
                "product_id": "nonexistent-id",
                "title": "X",
                "price": 100,
            }),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success());
}

#[tokio::test]
async fn non_admin_cannot_create_variant() {
    let (mut app, _, _, product_id) = setup_with_product().await;
    let (access_token, _) = register_and_login(
        &mut app,
        "variant_user@test.com",
        "variantuser",
        "Password123!",
    )
    .await;

    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/product-variants",
            json!({"product_id": product_id, "title": "X", "price": 100}),
            &access_token,
        ),
    )
    .await;
    assert!(!status.is_success());
}

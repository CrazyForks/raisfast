use super::*;

async fn admin_token() -> String {
    let pool = test_pool_with_tenants().await;
    let (int_id, id) = create_admin(&pool).await;
    make_token(&id, int_id, raisfast::models::user::UserRole::Admin)
}

#[tokio::test]
async fn create_and_get_tenant() {
    let (mut app, _) = test_app_with_tenants().await;
    let tok = admin_token().await;

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/tenants",
            json!({"name": "Acme Corp", "domain": "acme.example.com"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create tenant: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Acme Corp");
    let id = body["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/tenants/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "get tenant: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Acme Corp");
}

#[tokio::test]
async fn update_tenant() {
    let (mut app, _) = test_app_with_tenants().await;
    let tok = admin_token().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/tenants", json!({"name": "Original"}), &tok),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/tenants/{id}"),
            json!({"name": "Updated Corp", "domain": "updated.example.com"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update tenant: {status} {body:?}");
    assert_eq!(body["data"]["name"], "Updated Corp");
}

#[tokio::test]
async fn delete_tenant() {
    let (mut app, _) = test_app_with_tenants().await;
    let tok = admin_token().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/tenants", json!({"name": "ToDelete"}), &tok),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/tenants/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete tenant: {status}");

    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/tenants/{id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_tenants() {
    let (mut app, _) = test_app_with_tenants().await;
    let tok = admin_token().await;

    send(
        &mut app,
        post_json_auth("/api/v1/admin/tenants", json!({"name": "Tenant A"}), &tok),
    )
    .await;
    send(
        &mut app,
        post_json_auth("/api/v1/admin/tenants", json!({"name": "Tenant B"}), &tok),
    )
    .await;

    let (status, body) = send(&mut app, get_auth("/api/v1/admin/tenants", &tok)).await;
    assert!(status.is_success(), "list tenants: {status} {body:?}");
    assert!(body["data"]["total"].as_i64().unwrap() >= 2);
}

#[tokio::test]
async fn get_tenant_not_found() {
    let (mut app, _) = test_app_with_tenants().await;
    let tok = admin_token().await;
    let fake_id = "9999999999999".to_string();
    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/tenants/{fake_id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_tenant_not_found() {
    let (mut app, _) = test_app_with_tenants().await;
    let tok = admin_token().await;
    let fake_id = "9999999999999".to_string();
    let (status, _) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/tenants/{fake_id}"),
            json!({"name": "Ghost"}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_tenant_not_found_is_idempotent() {
    let (mut app, _) = test_app_with_tenants().await;
    let tok = admin_token().await;
    let fake_id = "9999999999999".to_string();
    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/tenants/{fake_id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete tenant is idempotent: {status}");
}

use super::*;

async fn admin_token() -> String {
    let pool = test_pool().await;
    let (int_id, id) = create_admin(&pool).await;
    make_token(&id, int_id, raisfast::models::user::UserRole::Admin)
}

#[tokio::test]
async fn list_roles() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/rbac/roles", &tok)).await;
    assert!(status.is_success(), "list roles: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert!(body["data"]["items"].is_array());
}

#[tokio::test]
async fn create_role_returns_in_list() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let role_name = format!("editor-{}", &uuid::Uuid::now_v7().to_string()[..8]);

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/rbac/roles",
            json!({"name": role_name, "description": "Editor role"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create role: {status} {body:?}");
    assert_eq!(body["data"]["name"], role_name);
    let role_id = body["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = send(&mut app, get_auth("/api/v1/admin/rbac/roles", &tok)).await;
    assert!(status.is_success(), "list roles: {status} {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    let found = items.iter().any(|r| r["id"] == role_id);
    assert!(found, "created role should appear in list");
}

#[tokio::test]
async fn update_role() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/rbac/roles",
            json!({"name": format!("mod-{}", &uuid::Uuid::now_v7().to_string()[..8])}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let new_name = format!("super-{}", &uuid::Uuid::now_v7().to_string()[..8]);
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/rbac/roles/{id}"),
            json!({"name": new_name, "description": "updated"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update role: {status} {body:?}");
    assert_eq!(body["data"]["name"], new_name);
}

#[tokio::test]
async fn delete_role() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;

    let role_name = format!("del-{}", &uuid::Uuid::now_v7().to_string()[..8]);
    let (_, create_body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/rbac/roles", json!({"name": role_name}), &tok),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/rbac/roles/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete role: {status}");

    let (status, body) = send(&mut app, get_auth("/api/v1/admin/rbac/roles", &tok)).await;
    assert!(status.is_success());
    let items = body["data"]["items"].as_array().unwrap();
    let found = items.iter().any(|r| r["id"] == id);
    assert!(!found, "deleted role should not appear in list");
}

#[tokio::test]
async fn set_and_get_permissions() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;

    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/rbac/roles",
            json!({"name": format!("perm-{}", &uuid::Uuid::now_v7().to_string()[..8])}),
            &tok,
        ),
    )
    .await;
    let role_id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/rbac/roles/{role_id}/permissions"),
            json!({"permissions": [
                {"action": "read", "subject": "posts"},
                {"action": "write", "subject": "comments", "conditions": {"own": "true"}}
            ]}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "set perms: {status} {body:?}");

    let (status, body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/rbac/roles/{role_id}/permissions"),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "get perms: {status} {body:?}");
    let perms = body["data"].as_array().unwrap();
    assert_eq!(perms.len(), 2);
}

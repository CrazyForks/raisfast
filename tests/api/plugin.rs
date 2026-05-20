use super::*;

async fn admin_token() -> String {
    let pool = test_pool().await;
    let (int_id, id) = create_admin(&pool).await;
    make_token(&id, int_id, raisfast::models::user::UserRole::Admin)
}

#[tokio::test]
async fn list_plugins_empty() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/plugins", &tok)).await;
    assert!(status.is_success(), "list plugins: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["total"], 0);
}

#[tokio::test]
async fn get_nonexistent_plugin() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let fake_id = "nonexistent-plugin";
    let (status, _body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/plugins/{fake_id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enable_nonexistent_plugin() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, _body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/plugins/ghost/enable", json!({}), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disable_nonexistent_plugin() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, _body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/plugins/ghost/disable", json!({}), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reload_nonexistent_plugin() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, _body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/plugins/ghost/reload", json!({}), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn remove_nonexistent_plugin_is_idempotent() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, _body) = send(&mut app, delete_auth("/api/v1/admin/plugins/ghost", &tok)).await;
    assert!(status.is_success(), "remove is idempotent: {status}");
}

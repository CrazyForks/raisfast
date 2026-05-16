use super::*;

fn admin_token() -> String {
    let id = uuid::Uuid::now_v7().to_string();
    make_token(&id, 1, raisfast::models::user::UserRole::Admin)
}

#[tokio::test]
async fn public_options_empty() {
    let (mut app, _) = test_app().await;
    let (status, body) = send(&mut app, get_req("/api/v1/options/public")).await;
    assert!(status.is_success(), "public options: {status} {body:?}");
    assert_eq!(body["code"], 0);
}

#[tokio::test]
async fn list_options_empty() {
    let (mut app, _) = test_app().await;
    let tok = admin_token();
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/options", &tok)).await;
    assert!(status.is_success(), "list options: {status} {body:?}");
    assert_eq!(body["code"], 0);
}

#[tokio::test]
async fn set_get_delete_option() {
    let (mut app, _) = test_app().await;
    let tok = admin_token();

    let key = format!("test.{}", uuid::Uuid::now_v7());

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/options/{key}"),
            json!({"value": "Test Value"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "set option: {status} {body:?}");

    let (status, _body) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/options/{key}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete option: {status}");
}

#[tokio::test]
async fn batch_update_options() {
    let (mut app, _) = test_app().await;
    let tok = admin_token();

    let key1 = format!("test.{}", &uuid::Uuid::now_v7().to_string()[..8]);
    let key2 = format!("test.{}", &uuid::Uuid::now_v7().to_string()[..8]);

    let (status, body) = send(
        &mut app,
        put_json_auth(
            "/api/v1/admin/options",
            json!({"options": {key1: "Val1", key2: "Val2"}}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "batch update: {status} {body:?}");
}

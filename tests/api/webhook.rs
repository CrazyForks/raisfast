use super::*;

async fn setup_admin() -> (axum::Router, String) {
    let admin_id = uuid::Uuid::now_v7().to_string();
    let token = make_token(&admin_id, 1, raisfast::models::user::UserRole::Admin);
    let (app, _) = test_app().await;
    (app, token)
}

#[tokio::test]
async fn list_empty() {
    let (mut app, tok) = setup_admin().await;
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/webhooks", &tok)).await;
    assert!(status.is_success(), "list: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["total"], 0);
    assert!(body["data"]["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn create_success() {
    let (mut app, tok) = setup_admin().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({
                "url": "https://example.com/hook",
                "events": ["post.created", "post.updated"],
                "description": "test hook"
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "create: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["url"], "https://example.com/hook");
    assert!(!body["data"]["secret"].as_str().unwrap().is_empty());
    assert_eq!(body["data"]["enabled"], true);
    assert!(!body["data"]["id"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn create_default_enabled_true() {
    let (mut app, tok) = setup_admin().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({
                "url": "https://example.com/default-enabled",
                "events": ["*"]
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["enabled"], true);
}

#[tokio::test]
async fn create_with_enabled_false() {
    let (mut app, tok) = setup_admin().await;
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({
                "url": "https://example.com/disabled",
                "events": ["post.created"],
                "enabled": false
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["enabled"], false);
}

#[tokio::test]
async fn create_validation_empty_url() {
    let (mut app, tok) = setup_admin().await;
    let (status, _body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "", "events": ["post.created"]}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty url should fail");
}

#[tokio::test]
async fn create_validation_empty_events() {
    let (mut app, tok) = setup_admin().await;
    let (status, _body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com", "events": []}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty events should fail");
}

#[tokio::test]
async fn create_requires_admin() {
    let (mut app, _state) = test_app().await;
    let author_id = uuid::Uuid::now_v7().to_string();
    let tok = make_token(&author_id, 1, raisfast::models::user::UserRole::Author);
    let (status, _body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com", "events": ["*"]}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "author should be forbidden");
}

#[tokio::test]
async fn get_by_id() {
    let (mut app, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com/get", "events": ["comment.created"]}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "get: {status} {body:?}");
    assert_eq!(body["data"]["id"], id);
    assert_eq!(body["data"]["url"], "https://example.com/get");
}

#[tokio::test]
async fn get_not_found() {
    let (mut app, tok) = setup_admin().await;
    let fake_id = "9999999999999".to_string();
    let (status, _body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/webhooks/{fake_id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_changes_fields() {
    let (mut app, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com/old", "events": ["post.created"]}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/webhooks/{id}"),
            json!({"url": "https://example.com/new", "description": "updated desc", "enabled": false}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "update: {status} {body:?}");
    assert_eq!(body["data"]["url"], "https://example.com/new");
    assert_eq!(body["data"]["description"], "updated desc");
    assert_eq!(body["data"]["enabled"], false);
}

#[tokio::test]
async fn update_events() {
    let (mut app, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com/ev", "events": ["post.created"]}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/webhooks/{id}"),
            json!({"events": ["post.created", "post.updated", "comment.created"]}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    let events: Vec<String> =
        serde_json::from_str(body["data"]["events"].as_str().unwrap()).unwrap();
    assert_eq!(events.len(), 3);
    assert!(events.contains(&"post.created".to_string()));
    assert!(events.contains(&"post.updated".to_string()));
    assert!(events.contains(&"comment.created".to_string()));
}

#[tokio::test]
async fn update_partial_only_description() {
    let (mut app, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com/partial", "events": ["*"]}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();
    let original_url = create_body["data"]["url"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/webhooks/{id}"),
            json!({"description": "partial update"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["url"], original_url);
    assert_eq!(body["data"]["description"], "partial update");
}

#[tokio::test]
async fn delete_success() {
    let (mut app, tok) = setup_admin().await;
    let (_, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com/del", "events": ["*"]}),
            &tok,
        ),
    )
    .await;
    let id = create_body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
    )
    .await;
    assert!(status.is_success(), "delete: {status} {body:?}");

    let (status, _body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "should be deleted");
}

#[tokio::test]
async fn delete_not_found() {
    let (mut app, tok) = setup_admin().await;
    let fake_id = "9999999999999".to_string();
    let (status, _body) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/webhooks/{fake_id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_not_found() {
    let (mut app, tok) = setup_admin().await;
    let fake_id = "9999999999999".to_string();
    let (status, _body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/webhooks/{fake_id}"),
            json!({"url": "https://example.com/ghost"}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_with_pagination() {
    let (mut app, tok) = setup_admin().await;
    for i in 0..5 {
        send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/webhooks",
                json!({"url": format!("https://example.com/page-{i}"), "events": ["*"]}),
                &tok,
            ),
        )
        .await;
    }

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/webhooks?page=1&page_size=3", &tok),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 3);
    assert_eq!(body["data"]["total"], 5);

    let (status, body2) = send(
        &mut app,
        get_auth("/api/v1/admin/webhooks?page=2&page_size=3", &tok),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body2["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(body2["data"]["total"], 5);
}

#[tokio::test]
async fn secret_is_hex_64_chars() {
    let (mut app, tok) = setup_admin().await;
    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({"url": "https://example.com/secret", "events": ["*"]}),
            &tok,
        ),
    )
    .await;
    let secret = body["data"]["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 64, "secret should be 64 hex chars (32 bytes)");
    assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn full_lifecycle_create_read_update_delete() {
    let (mut app, tok) = setup_admin().await;

    let (status, create_body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/webhooks",
            json!({
                "url": "https://example.com/lifecycle",
                "events": ["post.created"],
                "description": "lifecycle test"
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    let id = create_body["data"]["id"].as_str().unwrap().to_string();
    let secret = create_body["data"]["secret"].as_str().unwrap().to_string();

    let (status, get_body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(get_body["data"]["secret"], secret.as_str());

    let (status, update_body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/webhooks/{id}"),
            json!({"enabled": false, "url": "https://example.com/lifecycle-v2"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(
        update_body["data"]["url"],
        "https://example.com/lifecycle-v2"
    );
    assert_eq!(update_body["data"]["enabled"], false);
    assert_eq!(
        update_body["data"]["secret"],
        secret.as_str(),
        "secret should not change on update"
    );

    let (status, list_body) = send(&mut app, get_auth("/api/v1/admin/webhooks", &tok)).await;
    assert!(status.is_success());
    assert_eq!(list_body["data"]["total"], 1);

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
    )
    .await;
    assert!(status.is_success());

    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/webhooks/{id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, list_body) = send(&mut app, get_auth("/api/v1/admin/webhooks", &tok)).await;
    assert!(status.is_success());
    assert_eq!(list_body["data"]["total"], 0);
}

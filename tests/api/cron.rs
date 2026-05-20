use super::*;

async fn cron_app() -> (axum::Router, AppState) {
    test_app().await
}

#[tokio::test]
async fn list_returns_empty() {
    let (int_id, id) = create_admin_helper().await;
    let (mut app, _) = cron_app().await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/crons", &tok)).await;
    assert!(status.is_success());
    assert_eq!(body["code"], 0);
    assert!(body["data"]["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn create_and_get() {
    let (int_id, id) = create_admin_helper().await;
    let (mut app, _) = cron_app().await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/crons",
            json!({
                "label": "Test Sitemap",
                "job_type": "generate_sitemap",
                "cron_expr": "0 0 */6 * * *",
                "enabled": true
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["label"], "Test Sitemap");
    assert_eq!(body["data"]["job_type"], "generate_sitemap");
    assert!(body["data"]["enabled"].as_bool().unwrap());

    let id = body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/crons/{id}"), &tok),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["id"], id);
}

#[tokio::test]
async fn update_changes_fields() {
    let (int_id, id) = create_admin_helper().await;
    let (mut app, _) = cron_app().await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/crons",
            json!({
                "label": "Original",
                "job_type": "my_task",
                "cron_expr": "0 0 * * * *",
                "enabled": true
            }),
            &tok,
        ),
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/crons/{id}"),
            json!({"label": "Updated", "enabled": false}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["label"], "Updated");
    assert!(!body["data"]["enabled"].as_bool().unwrap());
}

#[tokio::test]
async fn toggle_enables_and_disables() {
    let (int_id, id) = create_admin_helper().await;
    let (mut app, _) = cron_app().await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/crons",
            json!({
                "label": "Toggle Test",
                "job_type": "test_task",
                "cron_expr": "0 0 * * * *",
                "enabled": true
            }),
            &tok,
        ),
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/crons/{id}/toggle"),
            json!({"enabled": false}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success());

    let (_, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/crons/{id}"), &tok),
    )
    .await;
    assert!(!body["data"]["enabled"].as_bool().unwrap());
}

#[tokio::test]
async fn delete_removes_schedule() {
    let (int_id, id) = create_admin_helper().await;
    let (mut app, _) = cron_app().await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (_, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/crons",
            json!({
                "label": "To Delete",
                "job_type": "delete_me",
                "cron_expr": "0 0 * * * *",
                "enabled": true
            }),
            &tok,
        ),
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap();

    let (status, _) = send(
        &mut app,
        Request::builder()
            .method("DELETE")
            .uri(format!("/api/v1/admin/crons/{id}"))
            .header(header::AUTHORIZATION, format!("Bearer {tok}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(status.is_success());

    let (_, list_body) = send(&mut app, get_auth("/api/v1/admin/crons", &tok)).await;
    let items = list_body["data"]["items"].as_array().unwrap();
    assert!(items.iter().all(|s| s["id"] != id));
}

#[tokio::test]
async fn logs_returns_empty_initially() {
    let (int_id, id) = create_admin_helper().await;
    let (mut app, _) = cron_app().await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (status, body) = send(&mut app, get_auth("/api/v1/admin/crons/logs", &tok)).await;
    assert!(status.is_success());
    assert!(body["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn validation_rejects_empty_label() {
    let (int_id, id) = create_admin_helper().await;
    let (mut app, _) = cron_app().await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/crons",
            json!({
                "label": "",
                "job_type": "test",
                "cron_expr": "0 0 * * * *",
                "enabled": true
            }),
            &tok,
        ),
    )
    .await;
    assert!(!status.is_success() || body["code"] != 0);
}

async fn create_admin_helper() -> (i64, String) {
    let pool = test_pool().await;
    create_admin(&pool).await
}

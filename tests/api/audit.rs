use super::*;

async fn admin_token() -> String {
    let pool = test_pool().await;
    let id = create_admin(&pool).await;
    make_token(&id, "admin")
}

#[tokio::test]
async fn list_audit_empty() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/audit", &tok)).await;
    assert!(status.is_success(), "list audit: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["total"], 0);
}

#[tokio::test]
async fn get_audit_not_found() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let fake_id = uuid::Uuid::now_v7().to_string();
    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/audit/{fake_id}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_audit_with_filter() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, body) = send(
        &mut app,
        get_auth(
            "/api/v1/admin/audit?action=create&page=1&page_size=10",
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "audit filter: {status} {body:?}");
    assert_eq!(body["code"], 0);
}

#[tokio::test]
async fn audit_pagination() {
    let (mut app, _) = test_app().await;
    let tok = admin_token().await;
    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/audit?page=2&page_size=5", &tok),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["page"], 2);
    assert_eq!(body["data"]["page_size"], 5);
}

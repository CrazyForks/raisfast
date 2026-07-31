use super::*;

async fn setup_admin() -> (axum::Router, String) {
    let (app, state) = test_app().await;
    let (int_id, id) = create_admin(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Admin);
    (app, tok)
}

#[tokio::test]
async fn overview_empty() {
    let (mut app, tok) = setup_admin().await;
    let (status, body) = send(&mut app, get_auth("/api/v1/admin/stats", &tok)).await;
    assert!(status.is_success(), "stats overview: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert!(body["data"]["total_posts"].is_number());
    assert!(body["data"]["total_users"].is_number());
}

#[tokio::test]
async fn content_stats() {
    let (mut app, tok) = setup_admin().await;
    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/stats/content/posts", &tok),
    )
    .await;
    assert!(status.is_success(), "content stats: {status} {body:?}");
    assert_eq!(body["code"], 0);
}

#[tokio::test]
async fn trends() {
    let (mut app, tok) = setup_admin().await;
    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/stats/trends?table=posts&days=7", &tok),
    )
    .await;
    assert!(status.is_success(), "trends: {status} {body:?}");
    assert_eq!(body["code"], 0);
}

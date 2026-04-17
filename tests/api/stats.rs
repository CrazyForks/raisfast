use super::*;

#[tokio::test]
async fn overview_empty() {
    let (mut app, _) = test_app().await;
    let (status, body) = send(&mut app, get_req("/api/v1/admin/stats")).await;
    assert!(status.is_success(), "stats overview: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert!(body["data"]["total_posts"].is_number());
    assert!(body["data"]["total_users"].is_number());
}

#[tokio::test]
async fn content_stats() {
    let (mut app, _) = test_app().await;
    let (status, body) = send(&mut app, get_req("/api/v1/admin/stats/content/posts")).await;
    assert!(status.is_success(), "content stats: {status} {body:?}");
    assert_eq!(body["code"], 0);
}

#[tokio::test]
async fn trends() {
    let (mut app, _) = test_app().await;
    let (status, body) = send(
        &mut app,
        get_req("/api/v1/admin/stats/trends?table=posts&days=7"),
    )
    .await;
    assert!(status.is_success(), "trends: {status} {body:?}");
    assert_eq!(body["code"], 0);
}

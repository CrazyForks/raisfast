use super::*;

#[tokio::test]
async fn sse_endpoint_returns_event_stream() {
    let (app, _) = test_app().await;
    let req = Request::builder()
        .uri("/api/v1/events")
        .header("accept", "text/event-stream")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    assert_eq!(
        resp.headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap()),
        Some("text/event-stream")
    );
    assert!(resp.headers().get("cache-control").is_some());
}

#[tokio::test]
async fn sse_endpoint_with_filter_param() {
    let (app, _) = test_app().await;
    let req = Request::builder()
        .uri("/api/v1/events?filter=PostCreated,CommentCreated")
        .header("accept", "text/event-stream")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    assert_eq!(
        resp.headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap()),
        Some("text/event-stream")
    );
}

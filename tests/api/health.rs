use super::*;

#[tokio::test]
async fn health_returns_ok() {
    let (mut app, _) = test_app().await;
    let (status, body): (StatusCode, Value) = send(&mut app, get_req("/health")).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["status"], "ok");
    assert_eq!(body["data"]["db"], "ok");
}

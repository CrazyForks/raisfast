use super::*;

#[tokio::test]
async fn health_returns_ok() {
    let (mut app, _) = test_app().await;
    let (status, body): (StatusCode, Value) = send(&mut app, get_req("/health")).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["status"], "UP");
    assert!(body["data"]["components"]["database"]["status"] == "UP");
    assert!(body["data"]["components"]["search"]["status"] == "UP");
    assert!(body["data"]["components"]["storage"]["status"] == "UP");
    assert!(body["data"]["components"]["cache"]["status"] == "UP");
    assert!(body["data"]["version"].is_string());
}

use super::*;

#[tokio::test]
async fn feed_empty() {
    let (mut app, _) = test_app().await;
    let (status, bytes) = send_raw(&mut app, get_req("/feed.xml")).await;
    assert!(status.is_success());
    let body = String::from_utf8(bytes).unwrap();
    assert!(body.contains("<rss"));
    assert!(body.contains("</rss>"));
}

#[tokio::test]
async fn feed_with_posts() {
    let (mut app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    let _: (StatusCode, Value) = send(
        &mut app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "RSS Post", "content": "content", "status": "published"}),
            &tok,
        ),
    )
    .await;
    let (status, bytes) = send_raw(&mut app, get_req("/feed.xml")).await;
    assert!(status.is_success());
    let body = String::from_utf8(bytes).unwrap();
    assert!(body.contains("RSS Post"));
}

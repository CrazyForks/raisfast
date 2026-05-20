use super::*;

#[tokio::test]
async fn upload_requires_auth() {
    let (mut app, _) = test_app().await;
    let boundary = "----b";
    let body_str = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"t.png\"\r\n\r\nx\r\n--{boundary}--\r\n"
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/media/upload")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body_str))
        .unwrap();
    let (status, _): (StatusCode, Value) = send(&mut app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upload_success() {
    let (mut app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);

    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let png_header = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    let body_bytes = {
        let mut v = Vec::new();
        v.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        v.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"test.png\"\r\n",
        );
        v.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
        v.extend_from_slice(png_header);
        v.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        v
    };

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/media/upload")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header(header::AUTHORIZATION, format!("Bearer {tok}"))
        .body(Body::from(body_bytes))
        .unwrap();

    let (status, body): (StatusCode, Value) = send(&mut app, req).await;
    assert!(status.is_success(), "upload failed: {status} {body:?}");
    assert_eq!(body["code"], 0);
    assert!(body["data"]["id"].is_string());
    assert!(body["data"]["url"].is_string());
}

#[tokio::test]
async fn list_requires_auth() {
    let (mut app, _) = test_app().await;
    let (status, _): (StatusCode, Value) = send(&mut app, get_req("/api/v1/media")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_success() {
    let (mut app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    let (status, body): (StatusCode, Value) = send(&mut app, get_auth("/api/v1/media", &tok)).await;
    assert!(status.is_success());
    assert!(body["data"]["items"].is_array());
}

#[tokio::test]
async fn delete_not_found() {
    let (mut app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    let fake = "nonexistent";
    let (status, _): (StatusCode, Value) = send(
        &mut app,
        delete_auth(&format!("/api/v1/media/{fake}"), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

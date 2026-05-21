use super::*;
use raisfast::DbDriver;

async fn setup_with_post() -> (axum::Router, AppState, String, String) {
    let (mut app, state) = test_app().await;
    let (int_id, id) = create_author(&state.pool).await;
    let tok = make_token(&id, int_id, raisfast::models::user::UserRole::Author);
    let slug = create_published_post(&mut app, &tok).await;
    (app, state, tok, slug)
}

#[tokio::test]
async fn guest_comment_success() {
    let (mut app, _, _, slug) = setup_with_post().await;
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            &format!("/api/v1/posts/{slug}/comments"),
            json!({"content": "Nice!", "nickname": "Guest1", "email": "g@test.com"}),
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["content"], "Nice!");
    assert_eq!(body["data"]["nickname"], "Guest1");
    assert!(body["data"]["created_by"].is_null());
}

#[tokio::test]
async fn guest_comment_requires_nickname() {
    let (mut app, _, _, slug) = setup_with_post().await;
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            &format!("/api/v1/posts/{slug}/comments"),
            json!({"content": "no nick"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 40000);
}

#[tokio::test]
async fn authed_comment_success() {
    let (mut app, _, _, slug) = setup_with_post().await;
    let (tok, _) = register_and_login(&mut app, "cmtr@test.com", "cmtr", "Password123").await;
    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/posts/{slug}/comments/authed"),
            json!({"content": "Auth comment"}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["content"], "Auth comment");
}

#[tokio::test]
async fn nested_comment() {
    let (mut app, state, _, slug) = setup_with_post().await;
    let (_, b1): (StatusCode, Value) = send(
        &mut app,
        post_json(
            &format!("/api/v1/posts/{slug}/comments"),
            json!({"content": "Parent", "nickname": "G1"}),
        ),
    )
    .await;
    let pid_str = b1["data"]["id"].as_str().unwrap();
    let pid: i64 = pid_str.parse().unwrap();

    let approve_sql = format!(
        "UPDATE comments SET status = 'approved' WHERE id = {}",
        raisfast::db::Driver::ph(1)
    );
    sqlx::query(&approve_sql)
        .bind(pid)
        .execute(&state.pool)
        .await
        .unwrap();

    let (status, body): (StatusCode, Value) = send(
        &mut app,
        post_json(
            &format!("/api/v1/posts/{slug}/comments"),
            json!({"content": "Reply", "nickname": "G2", "parent_id": pid.to_string()}),
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert!(body["data"]["id"].is_string());
}

#[tokio::test]
async fn list_comments() {
    let (mut app, state, _, slug) = setup_with_post().await;
    let _: (StatusCode, Value) = send(
        &mut app,
        post_json(
            &format!("/api/v1/posts/{slug}/comments"),
            json!({"content": "C1", "nickname": "A"}),
        ),
    )
    .await;
    let _: (StatusCode, Value) = send(
        &mut app,
        post_json(
            &format!("/api/v1/posts/{slug}/comments"),
            json!({"content": "C2", "nickname": "B"}),
        ),
    )
    .await;

    sqlx::query("UPDATE comments SET status = 'approved' WHERE status = 'pending'")
        .execute(&state.pool)
        .await
        .unwrap();

    let (status, body): (StatusCode, Value) =
        send(&mut app, get_req(&format!("/api/v1/posts/{slug}/comments"))).await;
    assert!(status.is_success());
    let items = body["data"]["items"].as_array().unwrap();
    assert!(items.len() >= 2);
}

#[tokio::test]
async fn delete_own_comment() {
    let (mut app, _, _, slug) = setup_with_post().await;
    let (tok, _) = register_and_login(&mut app, "delc@test.com", "delcuser", "Password123").await;
    let (_, b): (StatusCode, Value) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/posts/{slug}/comments/authed"),
            json!({"content": "mine"}),
            &tok,
        ),
    )
    .await;
    let cid = b["data"]["id"].as_str().unwrap().to_string();
    let (status, _): (StatusCode, Value) = send(
        &mut app,
        delete_auth(&format!("/api/v1/comments/{cid}"), &tok),
    )
    .await;
    assert!(status.is_success());
}

#[tokio::test]
async fn delete_not_owner_forbidden() {
    let (mut app, _, _, slug) = setup_with_post().await;
    let (t1, _) = register_and_login(&mut app, "own@test.com", "own", "Password123").await;
    let (t2, _) = register_and_login(&mut app, "str@test.com", "str", "Password123").await;
    let (_, b): (StatusCode, Value) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/posts/{slug}/comments/authed"),
            json!({"content": "x"}),
            &t1,
        ),
    )
    .await;
    let cid = b["data"]["id"].as_str().unwrap().to_string();
    let (status, _): (StatusCode, Value) = send(
        &mut app,
        delete_auth(&format!("/api/v1/comments/{cid}"), &t2),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_status_admin() {
    let (mut app, state, _, slug) = setup_with_post().await;
    let _: (StatusCode, Value) = send(
        &mut app,
        post_json(
            &format!("/api/v1/posts/{slug}/comments"),
            json!({"content": "mod me", "nickname": "G"}),
        ),
    )
    .await;

    let cid_i64: i64 = sqlx::query_scalar("SELECT id FROM comments WHERE content = 'mod me'")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    let cid = cid_i64.to_string();

    let (admin_int_id, admin_id) = create_admin(&state.pool).await;
    let admin_tok = make_token(
        &admin_id,
        admin_int_id,
        raisfast::models::user::UserRole::Admin,
    );
    let (status, _): (StatusCode, Value) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/comments/{cid}/status"),
            json!({"status": "approved"}),
            &admin_tok,
        ),
    )
    .await;
    assert!(status.is_success());
}

#[tokio::test]
async fn update_status_requires_admin() {
    let (mut app, _, _, _) = setup_with_post().await;
    let (tok, _) = register_and_login(&mut app, "na@test.com", "nauser", "Password123").await;
    let fake = "9999999999999";
    let (status, _): (StatusCode, Value) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/comments/{fake}/status"),
            json!({"status": "approved"}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

use super::*;

#[allow(dead_code)]
struct Ctx {
    app: axum::Router,
    state: AppState,
    tok: String,
    created_by: String,
    cat_id: String,
    tag_id: String,
}

async fn setup() -> Ctx {
    let (mut app, state) = test_app().await;
    let (author_int_id, author_doc_id) = create_author(&state.pool).await;
    let tok = make_token(&author_doc_id, author_int_id, "author");

    let (_, cb): (StatusCode, Value) = send(
        &mut app,
        post_json_auth("/api/v1/categories", json!({"name": "Tech"}), &tok),
    )
    .await;
    let cat_id = cb["data"]["id"].as_i64().unwrap().to_string();

    let (_, tb): (StatusCode, Value) = send(
        &mut app,
        post_json_auth("/api/v1/tags", json!({"name": "rust"}), &tok),
    )
    .await;
    let tag_id = tb["data"]["id"].as_i64().unwrap().to_string();

    Ctx {
        app,
        state,
        tok,
        created_by: author_doc_id,
        cat_id,
        tag_id,
    }
}

#[tokio::test]
async fn create_success() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({
                "title": "Hello Axum",
                "content": "# Hello\n**markdown**",
                "status": "published",
                "category_id": c.cat_id,
                "tag_ids": [c.tag_id],
            }),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["title"], "Hello Axum");
    assert_eq!(body["data"]["status"], "published");
    assert!(body["data"]["slug"].is_string());
    assert!(body["data"]["content"].is_string());
    assert_eq!(body["data"]["tags"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn create_requires_author() {
    let (mut app, _) = test_app().await;
    let (tok, _) = register_and_login(&mut app, "pr@test.com", "pruser", "Password123").await;
    let (status, _): (StatusCode, Value) = send(
        &mut app,
        post_json_auth("/api/v1/posts", json!({"title": "T", "content": "C"}), &tok),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_validation() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth("/api/v1/posts", json!({"title": "", "content": ""}), &c.tok),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 40000);
}

#[tokio::test]
async fn get_by_slug() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) =
        send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["title"], "Test Post");
}

#[tokio::test]
async fn view_count_increments() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (_, b1): (StatusCode, Value) =
        send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
    assert_eq!(b1["data"]["view_count"], 1);
    let (_, b2): (StatusCode, Value) =
        send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
    assert_eq!(b2["data"]["view_count"], 2);
}

#[tokio::test]
async fn get_not_found() {
    let (mut app, _) = test_app().await;
    let (status, _): (StatusCode, Value) = send(&mut app, get_req("/api/v1/posts/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_paginated() {
    let mut c = setup().await;
    for i in 1..=5u8 {
        let _: (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": format!("P{i}"), "content": "c", "status": "published"}),
                &c.tok,
            ),
        )
        .await;
    }
    let (status, body): (StatusCode, Value) =
        send(&mut c.app, get_req("/api/v1/posts?page=1&page_size=3")).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 3);
    assert!(body["data"]["total"].as_i64().unwrap() >= 5);
    assert_eq!(body["data"]["page"], 1);
    assert_eq!(body["data"]["page_size"], 3);
}

#[tokio::test]
async fn search() {
    let mut c = setup().await;
    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Rust Tips", "content": "Learn Rust", "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Go Tips", "content": "Learn Go", "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    let (_, body): (StatusCode, Value) = send(&mut c.app, get_req("/api/v1/posts?q=Rust")).await;
    let items = body["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Rust Tips");
}

#[tokio::test]
async fn update_success() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"title": "Updated", "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["title"], "Updated");
}

#[tokio::test]
async fn update_not_owner_forbidden() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (other, _) =
        register_and_login(&mut c.app, "other@test.com", "otheruser", "Password123").await;
    let (status, _): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"title": "Hack"}),
            &other,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_success() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, _): (StatusCode, Value) = send(
        &mut c.app,
        delete_auth(&format!("/api/v1/posts/{slug}"), &c.tok),
    )
    .await;
    assert!(status.is_success());
    let (s, _): (StatusCode, Value) =
        send(&mut c.app, get_req(&format!("/api/v1/posts/{slug}"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_can_delete_others() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let admin_id = create_admin(&c.state.pool).await;
    let admin_tok = make_token(&admin_id.1, admin_id.0, "admin");
    let (status, _): (StatusCode, Value) = send(
        &mut c.app,
        delete_auth(&format!("/api/v1/posts/{slug}"), &admin_tok),
    )
    .await;
    assert!(status.is_success());
}

#[tokio::test]
async fn filter_by_category() {
    let mut c = setup().await;
    let (_, cb): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth("/api/v1/categories", json!({"name": "Other"}), &c.tok),
    )
    .await;
    let other_cat = cb["data"]["id"].as_i64().unwrap().to_string();

    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "InTech", "content": "c", "status": "published", "category_id": c.cat_id}),
            &c.tok,
        ),
    )
    .await;
    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "InOther", "content": "c", "status": "published", "category_id": other_cat}),
            &c.tok,
        ),
    )
    .await;

    let (_, body): (StatusCode, Value) = send(
        &mut c.app,
        get_req(&format!("/api/v1/posts?category_id={}", c.cat_id)),
    )
    .await;
    let items = body["data"]["items"].as_array().unwrap();
    let cat_int: i64 = c.cat_id.parse().unwrap();
    assert!(items.iter().all(|p| p["category_id"] == cat_int));
}

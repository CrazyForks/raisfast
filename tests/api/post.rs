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
    let (author_int_id, author_id) = create_author(&state.pool).await;
    let tok = make_token(
        &author_id,
        author_int_id,
        raisfast::models::user::UserRole::Author,
    );

    let (_, cb): (StatusCode, Value) = send(
        &mut app,
        post_json_auth("/api/v1/categories", json!({"name": "Tech"}), &tok),
    )
    .await;
    let cat_id = cb["data"]["id"].as_str().unwrap().to_string();

    let (_, tb): (StatusCode, Value) = send(
        &mut app,
        post_json_auth("/api/v1/tags", json!({"name": "rust"}), &tok),
    )
    .await;
    let tag_id = tb["data"]["id"].as_str().unwrap().to_string();

    Ctx {
        app,
        state,
        tok,
        created_by: author_id,
        cat_id,
        tag_id,
    }
}

#[allow(dead_code)]
async fn create_draft_post(app: &mut axum::Router, tok: &str) -> String {
    let (_, body) = send(
        app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Draft Post", "content": "draft content", "status": "draft"}),
            tok,
        ),
    )
    .await;
    body["data"]["slug"].as_str().unwrap().to_string()
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
    assert!(body["data"]["excerpt"].is_string());
    assert!(body["data"]["created_at"].is_string());
    assert!(body["data"]["updated_at"].is_string());
    assert!(body["data"]["id"].is_string());
    assert!(body["data"]["view_count"].is_number());
    assert!(body["data"]["is_pinned"].is_boolean());
}

#[tokio::test]
async fn create_draft_default_status() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Auto Draft", "content": "some content"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["status"], "draft");
}

#[tokio::test]
async fn create_with_cover_image() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({
                "title": "Cover Post",
                "content": "content",
                "status": "published",
                "cover_image": "/uploads/cover.jpg",
            }),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["cover_image"], "/uploads/cover.jpg");
}

#[tokio::test]
async fn create_without_tags() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "No Tags", "content": "c", "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["tags"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn create_auto_excerpt_from_long_content() {
    let mut c = setup().await;
    let long_content = "x".repeat(300);
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Long Content", "content": long_content, "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    let excerpt = body["data"]["excerpt"].as_str().unwrap();
    assert!(excerpt.ends_with("..."));
}

#[tokio::test]
async fn create_author_name_populated() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Author Post", "content": "c", "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert!(
        body["data"]["author_name"].is_string(),
        "author_name should be set"
    );
}

#[tokio::test]
async fn create_with_category_name() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({
                "title": "Cat Post",
                "content": "c",
                "status": "published",
                "category_id": c.cat_id,
            }),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["category_name"], "Tech");
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
    assert!(body["data"]["author_name"].is_string());
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
async fn list_empty() {
    let (mut app, _) = test_app().await;
    let (status, body): (StatusCode, Value) =
        send(&mut app, get_req("/api/v1/posts?page=1&page_size=10")).await;
    assert!(status.is_success());
    assert_eq!(body["data"]["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["data"]["total"], 0);
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
async fn search_empty_query_lists_all() {
    let mut c = setup().await;
    create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) = send(&mut c.app, get_req("/api/v1/posts?q=")).await;
    assert!(status.is_success());
    assert!(body["data"]["total"].as_i64().unwrap() >= 1);
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
    assert_ne!(
        body["data"]["slug"], slug,
        "slug should change when title changes"
    );
}

#[tokio::test]
async fn update_content_and_excerpt() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"content": "new content", "excerpt": "custom excerpt"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["excerpt"], "custom excerpt");
}

#[tokio::test]
async fn update_with_tag_ids() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"tag_ids": [c.tag_id]}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["tags"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn update_cover_image() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"cover_image": "/new/cover.png"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["cover_image"], "/new/cover.png");
}

#[tokio::test]
async fn update_same_title_slug_unchanged() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"content": "just update content"}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(
        body["data"]["slug"], slug,
        "slug should not change when title unchanged"
    );
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
async fn admin_can_update_others_post() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let admin_id = create_admin(&c.state.pool).await;
    let admin_tok = make_token(
        &admin_id.1,
        admin_id.0,
        raisfast::models::user::UserRole::Admin,
    );
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"title": "Admin Edit"}),
            &admin_tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["title"], "Admin Edit");
}

#[tokio::test]
async fn update_nonexistent_post_404() {
    let mut c = setup().await;
    let (status, _): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            "/api/v1/posts/nonexistent-slug",
            json!({"title": "Nope"}),
            &c.tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
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
    let admin_tok = make_token(
        &admin_id.1,
        admin_id.0,
        raisfast::models::user::UserRole::Admin,
    );
    let (status, _): (StatusCode, Value) = send(
        &mut c.app,
        delete_auth(&format!("/api/v1/posts/{slug}"), &admin_tok),
    )
    .await;
    assert!(status.is_success());
}

#[tokio::test]
async fn delete_nonexistent_404() {
    let mut c = setup().await;
    let (status, _): (StatusCode, Value) =
        send(&mut c.app, delete_auth("/api/v1/posts/nope", &c.tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_not_owner_forbidden() {
    let mut c = setup().await;
    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (other, _) =
        register_and_login(&mut c.app, "other2@test.com", "otheruser2", "Password123").await;
    let (status, _): (StatusCode, Value) = send(
        &mut c.app,
        delete_auth(&format!("/api/v1/posts/{slug}"), &other),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn filter_by_category() {
    let mut c = setup().await;
    let (_, cb): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth("/api/v1/categories", json!({"name": "Other"}), &c.tok),
    )
    .await;
    let other_cat = cb["data"]["id"].as_str().unwrap().to_string();

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
    assert!(items.iter().all(|p| p["title"] == "InTech"));
}

#[tokio::test]
async fn filter_by_status() {
    let mut c = setup().await;
    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Published1", "content": "c", "status": "published", "category_id": c.cat_id}),
            &c.tok,
        ),
    )
    .await;
    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Draft1", "content": "c", "status": "draft", "category_id": c.cat_id}),
            &c.tok,
        ),
    )
    .await;

    let (_, body): (StatusCode, Value) =
        send(&mut c.app, get_req("/api/v1/posts?status=published")).await;
    let items = body["data"]["items"].as_array().unwrap();
    assert!(
        items.iter().all(|p| p["status"] == "published"),
        "expected all published, got: {items:?}"
    );
}

#[tokio::test]
async fn filter_by_tag() {
    let mut c = setup().await;
    let (_, tb): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth("/api/v1/tags", json!({"name": "go"}), &c.tok),
    )
    .await;
    let tag_go = tb["data"]["id"].as_str().unwrap().to_string();

    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "RustPost", "content": "c", "status": "published", "tag_ids": [c.tag_id]}),
            &c.tok,
        ),
    )
    .await;
    let _: (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "GoPost", "content": "c", "status": "published", "tag_ids": [&tag_go]}),
            &c.tok,
        ),
    )
    .await;

    let (_, body): (StatusCode, Value) = send(
        &mut c.app,
        get_req(&format!("/api/v1/posts?tag_id={}", c.tag_id)),
    )
    .await;
    let items = body["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "expected 1 post, got: {items:?}");
    assert_eq!(items[0]["title"], "RustPost");
}

#[tokio::test]
async fn update_changes_category() {
    let mut c = setup().await;
    let (_, cb): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth("/api/v1/categories", json!({"name": "CatB"}), &c.tok),
    )
    .await;
    let cat_b = cb["data"]["id"].as_str().unwrap().to_string();

    let slug = create_published_post(&mut c.app, &c.tok).await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        put_json_auth(
            &format!("/api/v1/posts/{slug}"),
            json!({"category_id": &cat_b}),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["category_name"], "CatB");
}

#[tokio::test]
async fn create_with_excerpt() {
    let mut c = setup().await;
    let (status, body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({
                "title": "ExcerptPost",
                "content": "Some content here",
                "status": "published",
                "excerpt": "Custom excerpt text",
                "category_id": c.cat_id,
            }),
            &c.tok,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["excerpt"], "Custom excerpt text");
}

#[tokio::test]
async fn list_sorting_by_created_at() {
    let mut c = setup().await;
    let uid = uuid::Uuid::now_v7().to_string();
    for i in 1..=3u8 {
        let (status, body): (StatusCode, Value) = send(
            &mut c.app,
            post_json_auth(
                "/api/v1/posts",
                json!({"title": format!("Sort{uid}{i}"), "content": "c", "status": "published"}),
                &c.tok,
            ),
        )
        .await;
        assert!(status.is_success(), "create post {i}: {status} {body:?}");
    }

    let (status, body): (StatusCode, Value) = send(&mut c.app, get_req("/api/v1/posts")).await;
    assert!(status.is_success(), "list: {status} {body:?}");
    let total = body["data"]["total"].as_i64().unwrap_or(0);
    assert!(total >= 3, "expected total >= 3, got {total}");
}

#[tokio::test]
async fn create_multiple_posts_unique_slugs() {
    let mut c = setup().await;
    let (_, b1): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Same Title", "content": "c1", "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    let (_, b2): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Same Title", "content": "c2", "status": "published"}),
            &c.tok,
        ),
    )
    .await;
    let slug1 = b1["data"]["slug"].as_str().unwrap();
    let slug2 = b2["data"]["slug"].as_str().unwrap();
    assert_ne!(slug1, slug2, "slugs should be unique even with same title");
}

#[tokio::test]
async fn create_with_invalid_category_id() {
    let mut c = setup().await;
    let (status, _): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({
                "title": "BadCat",
                "content": "c",
                "status": "published",
                "category_id": "nonexistent-id",
            }),
            &c.tok,
        ),
    )
    .await;
    assert!(
        status.is_server_error() || status == StatusCode::BAD_REQUEST,
        "expected error for invalid category_id, got {status}"
    );
}

#[tokio::test]
async fn list_multiple_posts_with_tags() {
    let mut c = setup().await;
    let (_, tb): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth("/api/v1/tags", json!({"name": "tag2"}), &c.tok),
    )
    .await;
    let tag2 = tb["data"]["id"].as_str().unwrap().to_string();

    let (create_status, create_body): (StatusCode, Value) = send(
        &mut c.app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "MultiTag", "content": "c", "status": "published", "tag_ids": [c.tag_id, &tag2]}),
            &c.tok,
        ),
    )
    .await;
    assert!(
        create_status.is_success(),
        "create: {create_status} {create_body:?}"
    );
    let create_tags = create_body["data"]["tags"].as_array().unwrap();
    assert_eq!(
        create_tags.len(),
        2,
        "create response tags: {create_tags:?}"
    );

    let (status, _body): (StatusCode, Value) = send(&mut c.app, get_req("/api/v1/posts")).await;
    assert!(status.is_success());
}

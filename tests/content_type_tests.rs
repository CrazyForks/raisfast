//! Content Type 端到端集成测试
//!
//! 验证动态内容类型系统的完整链路：
//! Schema 解析 → Migration 建表 → CRUD → 租户隔离

use std::collections::HashMap;

use serde_json::json;

use rust_blog::content_type::ContentTypeRegistry;
use rust_blog::content_type::repository::{ContentQuery, ContentRepository};
use rust_blog::content_type::schema::ContentTypeSchema;
use rust_blog::db::tenant;

const PRODUCT_TOML: &str = r#"
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "ct_products"
description = "商品"
draft_publish = true
timestamps = true
soft_delete = false

[fields.title]
type = "text"
required = true
max_length = 200

[fields.slug]
type = "uid"
target_field = "title"
unique = true

[fields.price]
type = "integer"
required = true
default = 0

[fields.description]
type = "text"

[fields.in_stock]
type = "boolean"
default = true

[[indexes]]
fields = ["slug"]
unique = true
"#;

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    for sql in [
        include_str!("../migrations/001_init.sql"),
        include_str!("../migrations/002_add_indexes.sql"),
        include_str!("../migrations/009_options.sql"),
        include_str!("../migrations/010_rbac.sql"),
        include_str!("../migrations/011_tenants.sql"),
        include_str!("../migrations/016_content_revisions.sql"),
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    tenant::invalidate_cache().await;
    pool
}

fn parse_product() -> ContentTypeSchema {
    ContentTypeSchema::parse_from_str(PRODUCT_TOML).unwrap()
}

fn parse_article() -> ContentTypeSchema {
    ContentTypeSchema::parse_from_file(std::path::Path::new(
        "extensions/first-ext/content_types/article.toml",
    ))
    .unwrap()
}

#[tokio::test]
async fn schema_parse_product() {
    let ct = parse_product();
    assert_eq!(ct.name, "Product");
    assert_eq!(ct.singular, "product");
    assert_eq!(ct.plural, "products");
    assert_eq!(ct.table, "ct_products");
    assert!(ct.draft_publish);
    assert!(ct.timestamps);
    assert!(!ct.soft_delete);
    assert!(ct.fields.iter().any(|f| f.name == "title" && f.required));
    assert!(ct.fields.iter().any(|f| f.name == "price"));
}

#[tokio::test]
async fn schema_parse_article_toml() {
    let ct = parse_article();
    assert_eq!(ct.name, "Article");
    assert_eq!(ct.singular, "article");
    assert_eq!(ct.plural, "articles");
    assert_eq!(ct.table, "articles");
}

#[tokio::test]
async fn migrate_creates_table() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool.clone());

    repo.migrate(&ct).await.unwrap();

    let rows: Vec<(i32, String, String, i32, Option<String>, i32)> =
        sqlx::query_as("PRAGMA table_info(ct_products)")
            .fetch_all(&pool)
            .await
            .unwrap();

    let col_names: Vec<&str> = rows
        .iter()
        .map(|(_, name, _, _, _, _)| name.as_str())
        .collect();
    assert!(col_names.contains(&"id"));
    assert!(col_names.contains(&"title"));
    assert!(col_names.contains(&"slug"));
    assert!(col_names.contains(&"price"));
    assert!(col_names.contains(&"description"));
    assert!(col_names.contains(&"in_stock"));
    assert!(col_names.contains(&"status"));
    assert!(col_names.contains(&"created_at"));
    assert!(col_names.contains(&"updated_at"));
}

#[tokio::test]
async fn migrate_idempotent() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool.clone());

    repo.migrate(&ct).await.unwrap();
    repo.migrate(&ct).await.unwrap();
    repo.migrate(&ct).await.unwrap();

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ct_products")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn create_and_find_by_id() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(
            &ct,
            json!({
                "title": "Test Product",
                "slug": "test-product",
                "price": 99,
                "description": "A test product",
                "in_stock": true
            }),
            None,
        )
        .await
        .unwrap();

    assert_eq!(created["title"], "Test Product");
    assert_eq!(created["price"], 99);
    assert_eq!(created["status"], "draft");

    let found = repo
        .find_by_id(&ct, created["id"].as_str().unwrap(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found["title"], "Test Product");
    assert_eq!(found["id"], created["id"]);
}

#[tokio::test]
async fn create_sets_defaults() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(
            &ct,
            json!({
                "title": "Minimal",
                "slug": "minimal",
                "price": 0
            }),
            None,
        )
        .await
        .unwrap();

    assert_eq!(created["status"], "draft");
    assert!(created["in_stock"].is_boolean() || created["in_stock"].is_i64());
    assert!(created.get("created_at").is_some());
    assert!(created.get("updated_at").is_some());
}

#[tokio::test]
async fn find_by_slug() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    repo.create(
        &ct,
        json!({"title": "Slug Test", "slug": "slug-test", "price": 10}),
        None,
    )
    .await
    .unwrap();

    let found = repo
        .find_by_slug(&ct, "slug-test", Some("draft"), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found["title"], "Slug Test");
}

#[tokio::test]
async fn find_paginated() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    for i in 1..=15 {
        repo.create(
            &ct,
            json!({"title": format!("Item {i}"), "slug": format!("item-{i}"), "price": i}),
            None,
        )
        .await
        .unwrap();
    }

    let query = ContentQuery {
        page: 1,
        page_size: 10,
        sort: None,
        filters: HashMap::new(),
        status: None,
        search: None,
        fields: None,
        tenant_id: None,
        include: None,
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 15);
    assert_eq!(items.len(), 10);

    let query = ContentQuery {
        page: 2,
        page_size: 10,
        sort: None,
        filters: HashMap::new(),
        status: None,
        search: None,
        fields: None,
        tenant_id: None,
        include: None,
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 15);
    assert_eq!(items.len(), 5);
}

#[tokio::test]
async fn find_with_status_filter() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    let _draft = repo
        .create(
            &ct,
            json!({"title": "Draft", "slug": "draft", "price": 1, "status": "draft"}),
            None,
        )
        .await
        .unwrap();
    let _published = repo
        .create(
            &ct,
            json!({"title": "Published", "slug": "published", "price": 2, "status": "published"}),
            None,
        )
        .await
        .unwrap();

    let query = ContentQuery {
        page: 1,
        page_size: 20,
        sort: None,
        filters: HashMap::new(),
        status: Some("published".into()),
        search: None,
        fields: None,
        tenant_id: None,
        include: None,
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0]["title"], "Published");
}

#[tokio::test]
async fn update_changes_fields() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(
            &ct,
            json!({"title": "Original", "slug": "original", "price": 50}),
            None,
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let updated = repo
        .update(&ct, &id, json!({"title": "Updated", "price": 99}), None)
        .await
        .unwrap();

    assert_eq!(updated["title"], "Updated");
    assert_eq!(updated["price"], 99);
    assert_eq!(updated["slug"], "original");
}

#[tokio::test]
async fn delete_removes_record() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(
            &ct,
            json!({"title": "To Delete", "slug": "to-delete", "price": 1}),
            None,
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    repo.delete(&ct, &id, None).await.unwrap();

    let found = repo.find_by_id(&ct, &id, None).await.unwrap();
    assert!(found.is_none());

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ct_products")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn soft_delete_marks_record() {
    let ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_notes"
timestamps = true
soft_delete = true

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();

    let pool = setup_pool().await;
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(&ct, json!({"title": "Soft Delete Me"}), None)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    repo.delete(&ct, &id, None).await.unwrap();

    let row: Option<(String,)> = sqlx::query_as("SELECT deleted_at FROM ct_notes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&repo.pool)
        .await
        .unwrap();

    let deleted_at = row.unwrap().0;
    assert!(!deleted_at.is_empty());
}

#[tokio::test]
async fn registry_load_and_lookup() {
    let ct = parse_product();
    let registry = ContentTypeRegistry::new();
    registry.register(ct);

    assert_eq!(registry.len(), 1);
    assert!(registry.get("product").is_some());
    assert!(registry.get("nonexistent").is_none());
    assert!(registry.get_by_table("ct_products").is_some());
    assert!(!registry.is_empty());
}

#[tokio::test]
async fn tenant_isolation() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let a = repo
        .create(
            &ct,
            json!({"title": "Tenant A Product", "slug": "tenant-a", "price": 100}),
            Some("tenant_a"),
        )
        .await
        .unwrap();
    let b = repo
        .create(
            &ct,
            json!({"title": "Tenant B Product", "slug": "tenant-b", "price": 200}),
            Some("tenant_b"),
        )
        .await
        .unwrap();

    let id_a = a["id"].as_str().unwrap();
    let id_b = b["id"].as_str().unwrap();

    assert!(
        repo.find_by_id(&ct, id_a, Some("tenant_b"))
            .await
            .unwrap()
            .is_none(),
        "tenant_b should not see tenant_a's data"
    );
    assert!(
        repo.find_by_id(&ct, id_b, Some("tenant_a"))
            .await
            .unwrap()
            .is_none(),
        "tenant_a should not see tenant_b's data"
    );
    assert!(
        repo.find_by_id(&ct, id_a, Some("tenant_a"))
            .await
            .unwrap()
            .is_some(),
        "tenant_a should see own data"
    );

    let query = ContentQuery {
        page: 1,
        page_size: 20,
        sort: None,
        filters: HashMap::new(),
        status: None,
        search: None,
        fields: None,
        tenant_id: Some("tenant_a".into()),
        include: None,
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0]["title"], "Tenant A Product");
}

#[tokio::test]
async fn delete_respects_tenant() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let a = repo
        .create(
            &ct,
            json!({"title": "A", "slug": "a", "price": 1}),
            Some("tenant_a"),
        )
        .await
        .unwrap();
    let id_a = a["id"].as_str().unwrap();

    repo.delete(&ct, id_a, Some("tenant_b")).await.unwrap();

    assert!(
        repo.find_by_id(&ct, id_a, Some("tenant_a"))
            .await
            .unwrap()
            .is_some(),
        "tenant_b should not be able to delete tenant_a's data"
    );

    repo.delete(&ct, id_a, Some("tenant_a")).await.unwrap();
    assert!(
        repo.find_by_id(&ct, id_a, Some("tenant_a"))
            .await
            .unwrap()
            .is_none(),
        "tenant_a should be able to delete own data"
    );
}

#[tokio::test]
async fn find_with_custom_sort() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    for (title, price) in [("Alpha", 30), ("Beta", 10), ("Gamma", 20)] {
        repo.create(
            &ct,
            json!({"title": title, "slug": title.to_lowercase(), "price": price}),
            None,
        )
        .await
        .unwrap();
    }

    let query = ContentQuery {
        page: 1,
        page_size: 20,
        sort: Some("price:asc".into()),
        filters: HashMap::new(),
        status: None,
        search: None,
        fields: None,
        tenant_id: None,
        include: None,
    };
    let (items, _) = repo.find(&ct, query).await.unwrap();
    assert_eq!(items[0]["title"], "Beta");
    assert_eq!(items[1]["title"], "Gamma");
    assert_eq!(items[2]["title"], "Alpha");
}

#[tokio::test]
async fn find_with_field_filter() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    repo.create(
        &ct,
        json!({"title": "Expensive", "slug": "expensive", "price": 999}),
        None,
    )
    .await
    .unwrap();
    repo.create(
        &ct,
        json!({"title": "Cheap", "slug": "cheap", "price": 1}),
        None,
    )
    .await
    .unwrap();

    let mut filters = HashMap::new();
    filters.insert("title".into(), json!("Cheap"));

    let query = ContentQuery {
        page: 1,
        page_size: 20,
        sort: None,
        filters,
        status: None,
        search: None,
        fields: None,
        tenant_id: None,
        include: None,
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0]["title"], "Cheap");
}

#[tokio::test]
async fn partial_field_selection() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    repo.create(
        &ct,
        json!({"title": "Select", "slug": "select", "price": 42}),
        None,
    )
    .await
    .unwrap();

    let query = ContentQuery {
        page: 1,
        page_size: 20,
        sort: None,
        filters: HashMap::new(),
        status: None,
        search: None,
        fields: Some(vec!["title".into()]),
        tenant_id: None,
        include: None,
    };
    let (items, _) = repo.find(&ct, query).await.unwrap();
    let obj = items[0].as_object().unwrap();
    assert!(obj.contains_key("id"));
    assert!(obj.contains_key("title"));
    assert!(!obj.contains_key("price"));
}

#[tokio::test]
async fn create_auto_generates_id_and_timestamps() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    let result = repo
        .create(
            &ct,
            json!({"title": "Auto", "slug": "auto", "price": 1}),
            None,
        )
        .await
        .unwrap();

    assert!(result["id"].is_string());
    assert!(!result["id"].as_str().unwrap().is_empty());
    assert!(result.get("created_at").is_some());
    assert!(result.get("updated_at").is_some());
}

#[tokio::test]
async fn create_without_body_object_returns_error() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    let result = repo.create(&ct, json!("not an object"), None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn update_with_no_fields_returns_error() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(&ct, json!({"title": "X", "slug": "x", "price": 1}), None)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let result = repo
        .update(&ct, &id, json!({"nonexistent_field": "v"}), None)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn migrate_adds_columns_incrementally() {
    let pool = setup_pool().await;
    let repo = ContentRepository::new(pool.clone());

    let ct_v1 = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_notes_v2"
timestamps = true

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    repo.migrate(&ct_v1).await.unwrap();

    let ct_v2 = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_notes_v2"
timestamps = true

[fields.title]
type = "text"
required = true

[fields.body]
type = "text"

[fields.priority]
type = "integer"
default = 0
"#,
    )
    .unwrap();
    repo.migrate(&ct_v2).await.unwrap();

    let created = repo
        .create(
            &ct_v2,
            json!({"title": "V2", "body": "hello", "priority": 5}),
            None,
        )
        .await
        .unwrap();
    assert_eq!(created["body"], "hello");
    assert_eq!(created["priority"], 5);
}

// ── Versioning 测试 ─────────────────────────────────────────────

const VERSIONED_TOML: &str = r#"
[content_type]
name = "Article"
singular = "article"
plural = "articles"
table = "ct_versioned_articles"
timestamps = true
versioning = true

[fields.title]
type = "text"
required = true

[fields.content]
type = "text"

[fields.status]
type = "text"
default = "draft"
"#;

fn parse_versioned() -> ContentTypeSchema {
    ContentTypeSchema::parse_from_str(VERSIONED_TOML).unwrap()
}

#[tokio::test]
async fn versioning_flag_parsed() {
    let ct = parse_versioned();
    assert!(ct.versioning);
    assert_eq!(ct.singular, "article");
}

#[tokio::test]
async fn versioning_creates_revision_on_update() {
    let pool = setup_pool().await;
    let ct = parse_versioned();
    let repo = ContentRepository::new(pool.clone());

    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(
            &ct,
            json!({"title": "V1 Title", "content": "V1 Content"}),
            None,
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();

    let _updated = repo
        .update(
            &ct,
            id,
            json!({"title": "V2 Title", "content": "V2 Content"}),
            None,
        )
        .await
        .unwrap();

    let revisions = rust_blog::models::content_revision::list_revisions(&pool, "article", id)
        .await
        .unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].revision_number, 1);

    let rev =
        rust_blog::models::content_revision::get_revision(&pool, "article", id, &revisions[0].id)
            .await
            .unwrap()
            .unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&rev.snapshot).unwrap();
    assert_eq!(snapshot["title"], "V1 Title");
    assert_eq!(snapshot["content"], "V1 Content");
}

#[tokio::test]
async fn versioning_multiple_updates_create_multiple_revisions() {
    let pool = setup_pool().await;
    let ct = parse_versioned();
    let repo = ContentRepository::new(pool.clone());

    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(&ct, json!({"title": "Rev0"}), None)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();

    repo.update(&ct, id, json!({"title": "Rev1"}), None)
        .await
        .unwrap();
    repo.update(&ct, id, json!({"title": "Rev2"}), None)
        .await
        .unwrap();
    repo.update(&ct, id, json!({"title": "Rev3"}), None)
        .await
        .unwrap();

    let revisions = rust_blog::models::content_revision::list_revisions(&pool, "article", id)
        .await
        .unwrap();
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[0].revision_number, 3);
    assert_eq!(revisions[1].revision_number, 2);
    assert_eq!(revisions[2].revision_number, 1);
}

#[tokio::test]
async fn versioning_delete_cleans_up_revisions() {
    let pool = setup_pool().await;
    let ct = parse_versioned();
    let repo = ContentRepository::new(pool.clone());

    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(&ct, json!({"title": "Temp"}), None)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();

    repo.update(&ct, id, json!({"title": "Updated"}), None)
        .await
        .unwrap();

    let before = rust_blog::models::content_revision::list_revisions(&pool, "article", id)
        .await
        .unwrap();
    assert_eq!(before.len(), 1);

    repo.delete(&ct, id, None).await.unwrap();

    let after = rust_blog::models::content_revision::list_revisions(&pool, "article", id)
        .await
        .unwrap();
    assert!(after.is_empty());
}

#[tokio::test]
async fn versioning_no_revision_when_disabled() {
    let pool = setup_pool().await;
    let ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_no_versioning"
timestamps = true

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    assert!(!ct.versioning);

    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct).await.unwrap();

    let created = repo
        .create(&ct, json!({"title": "NoRev"}), None)
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();

    repo.update(&ct, id, json!({"title": "Updated"}), None)
        .await
        .unwrap();

    let revisions = rust_blog::models::content_revision::list_revisions(&pool, "note", id)
        .await
        .unwrap();
    assert!(revisions.is_empty());
}

#[tokio::test]
async fn versioning_diff_computes_correctly() {
    let old = json!({"title": "Old", "content": "Same", "status": "draft"});
    let new = json!({"title": "New", "content": "Same", "status": "published", "extra": 42});

    let diff = rust_blog::models::content_revision::compute_diff(&old, &new);

    let changed = diff["changed"].as_object().unwrap();
    assert!(changed.contains_key("title"));
    assert!(changed.contains_key("status"));
    assert_eq!(changed.len(), 2);

    let added = diff["added"].as_object().unwrap();
    assert!(added.contains_key("extra"));
    assert_eq!(added.len(), 1);

    let removed = diff["removed"].as_object().unwrap();
    assert!(removed.is_empty());
}

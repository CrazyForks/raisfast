//! Content Type 端到端集成测试
//!
//! 验证动态内容类型系统的完整链路：
//! Schema 解析 → Migration 建表 → CRUD → 租户隔离

use std::collections::HashMap;

use serde_json::json;

use raisfast::content_type::ContentTypeRegistry;
use raisfast::content_type::repository::{ContentQuery, ContentRepository, SaveContext};
use raisfast::content_type::schema::ContentTypeSchema;
use raisfast::db::tenant;
use raisfast::types::snowflake_id::SnowflakeId;

const PRODUCT_TOML: &str = r#"
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "ct_products"
description = "商品"
implements = ["ownable", "timestampable", "tenantable"]

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
    let pool = raisfast::db::Pool::connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query(raisfast::db::schema::SCHEMA_SQL)
        .execute(&pool)
        .await
        .unwrap();
    tenant::invalidate_cache().await;
    pool
}

fn parse_product() -> ContentTypeSchema {
    let mut ct = ContentTypeSchema::parse_from_str(PRODUCT_TOML).unwrap();
    cache_ct(&mut ct);
    ct
}

fn now_str() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn with_timestamps(data: serde_json::Value) -> serde_json::Value {
    let mut obj = data
        .as_object()
        .expect("with_timestamps: expected JSON object")
        .clone();
    let now = now_str();
    obj.insert("created_at".into(), serde_json::json!(now));
    obj.insert("updated_at".into(), serde_json::json!(now));
    serde_json::Value::Object(obj)
}

fn parse_article() -> ContentTypeSchema {
    ContentTypeSchema::parse_from_file(std::path::Path::new(
        "extensions/content_types/first-ext_article.toml",
    ))
    .unwrap()
}

fn test_ct_registry() -> raisfast::content_type::ContentTypeRegistry {
    raisfast::content_type::ContentTypeRegistry::new()
}

fn test_protocol_registry() -> raisfast::protocols::ProtocolRegistry {
    let mut reg = raisfast::protocols::ProtocolRegistry::new();
    reg.register(raisfast::protocols::ownable::OwnableProtocol);
    reg.register(raisfast::protocols::timestampable::TimestampableProtocol);
    reg.register(raisfast::protocols::soft_deletable::SoftDeletableProtocol);
    reg.register(raisfast::protocols::versionable::VersionableProtocol);
    reg.register(raisfast::protocols::lockable::LockableProtocol);
    reg.register(raisfast::protocols::sortable::SortableProtocol);
    reg.register(raisfast::protocols::expirable::ExpirableProtocol);
    reg.register(raisfast::protocols::nestable::NestableProtocol);
    reg.register(raisfast::protocols::tenantable::TenantableProtocol);
    reg
}

fn cache_ct(ct: &mut ContentTypeSchema) {
    ct.cache_protocol_columns(&test_protocol_registry());
}

#[tokio::test]
async fn schema_parse_product() {
    let ct = parse_product();
    assert_eq!(ct.name, "Product");
    assert_eq!(ct.singular, "product");
    assert_eq!(ct.plural, "products");
    assert_eq!(ct.table, "ct_products");
    assert!(!ct.implements_protocol("soft_deletable"));
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

    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

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
    assert!(col_names.contains(&"created_at"));
    assert!(col_names.contains(&"updated_at"));
}

#[tokio::test]
async fn migrate_idempotent() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool.clone());

    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({
                "title": "Test Product",
                "slug": "test-product",
                "price": 99,
                "description": "A test product",
                "in_stock": true
            })),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();

    assert_eq!(created["title"], "Test Product");
    assert_eq!(created["price"], 99);

    let found = repo
        .find_by_id(
            &ct,
            created["id"].as_str().unwrap().parse().unwrap(),
            None,
            true,
        )
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({
                "title": "Minimal",
                "slug": "minimal",
                "price": 0
            })),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();

    assert!(created["in_stock"].is_boolean() || created["in_stock"].is_i64());
    assert!(created.get("created_at").is_some());
    assert!(created.get("updated_at").is_some());
}

#[tokio::test]
async fn find_by_slug() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    repo.create(
        &ct,
        with_timestamps(json!({"title": "Slug Test", "slug": "slug-test", "price": 10})),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();

    let found = repo
        .find_by_slug(&ct, "slug-test", Some("draft"), None, true)
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    for i in 1..=15 {
        repo.create(
            &ct,
            with_timestamps(
                json!({"title": format!("Item {i}"), "slug": format!("item-{i}"), "price": i}),
            ),
            None,
            &SaveContext::default(),
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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 15);
    assert_eq!(items.len(), 5);
}

#[tokio::test]
async fn update_changes_fields() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Original", "slug": "original", "price": 50})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

    let updated = repo
        .update(
            &ct,
            raisfast::types::snowflake_id::SnowflakeId(id),
            json!({"title": "Updated", "price": 99}),
            None,
            &SaveContext::default(),
        )
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "To Delete", "slug": "to-delete", "price": 1})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

    repo.delete(
        &ct,
        raisfast::types::snowflake_id::SnowflakeId(id),
        None,
        &test_protocol_registry(),
        &test_ct_registry(),
    )
    .await
    .unwrap();

    let found = repo
        .find_by_id(&ct, SnowflakeId(id), None, true)
        .await
        .unwrap();
    assert!(found.is_none());

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ct_products")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn soft_delete_marks_record() {
    let mut ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_notes"
implements = ["ownable", "timestampable", "soft_deletable"]

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    cache_ct(&mut ct);

    let pool = setup_pool().await;
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Soft Delete Me"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

    repo.delete(
        &ct,
        raisfast::types::snowflake_id::SnowflakeId(id),
        None,
        &test_protocol_registry(),
        &test_ct_registry(),
    )
    .await
    .unwrap();

    let row: Option<(String,)> = sqlx::query_as("SELECT deleted_at FROM ct_notes WHERE id = ?")
        .bind(id)
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
    let reserved: Vec<&str> = Vec::new();
    let protocol_reg = test_protocol_registry();
    let protocol_names: Vec<&str> = protocol_reg.names();
    registry
        .register(
            ct,
            &raisfast::config::app::RuleEngineConfig::default(),
            &reserved,
            &protocol_names,
            &protocol_reg,
        )
        .unwrap();

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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let a = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Tenant A Product", "slug": "tenant-a", "price": 100})),
            Some("tenant_a"),
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let b = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Tenant B Product", "slug": "tenant-b", "price": 200})),
            Some("tenant_b"),
            &SaveContext::default(),
        )
        .await
        .unwrap();

    let id_a: i64 = a["id"].as_str().unwrap().parse().unwrap();
    let id_b: i64 = b["id"].as_str().unwrap().parse().unwrap();

    assert!(
        repo.find_by_id(&ct, SnowflakeId(id_a), Some("tenant_b"), true)
            .await
            .unwrap()
            .is_none(),
        "tenant_b should not see tenant_a's data"
    );
    assert!(
        repo.find_by_id(&ct, SnowflakeId(id_b), Some("tenant_a"), true)
            .await
            .unwrap()
            .is_none(),
        "tenant_a should not see tenant_b's data"
    );
    assert!(
        repo.find_by_id(&ct, SnowflakeId(id_a), Some("tenant_a"), true)
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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let a = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "A", "slug": "a", "price": 1})),
            Some("tenant_a"),
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id_a: i64 = a["id"].as_str().unwrap().parse().unwrap();

    repo.delete(
        &ct,
        raisfast::types::snowflake_id::SnowflakeId(id_a),
        Some("tenant_b"),
        &test_protocol_registry(),
        &test_ct_registry(),
    )
    .await
    .unwrap();

    assert!(
        repo.find_by_id(&ct, SnowflakeId(id_a), Some("tenant_a"), true)
            .await
            .unwrap()
            .is_some(),
        "tenant_b should not be able to delete tenant_a's data"
    );

    repo.delete(
        &ct,
        raisfast::types::snowflake_id::SnowflakeId(id_a),
        Some("tenant_a"),
        &test_protocol_registry(),
        &test_ct_registry(),
    )
    .await
    .unwrap();
    assert!(
        repo.find_by_id(&ct, SnowflakeId(id_a), Some("tenant_a"), true)
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    for (title, price) in [("Alpha", 30), ("Beta", 10), ("Gamma", 20)] {
        repo.create(
            &ct,
            with_timestamps(json!({"title": title, "slug": title.to_lowercase(), "price": price})),
            None,
            &SaveContext::default(),
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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    repo.create(
        &ct,
        with_timestamps(json!({"title": "Expensive", "slug": "expensive", "price": 999})),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();
    repo.create(
        &ct,
        with_timestamps(json!({"title": "Cheap", "slug": "cheap", "price": 1})),
        None,
        &SaveContext::default(),
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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    repo.create(
        &ct,
        with_timestamps(json!({"title": "Select", "slug": "select", "price": 42})),
        None,
        &SaveContext::default(),
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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
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
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let result = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Auto", "slug": "auto", "price": 1})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();

    assert!(result["id"].is_string());
    assert!(result.get("created_at").is_some());
    assert!(result.get("updated_at").is_some());
}

#[tokio::test]
async fn create_without_body_object_returns_error() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let result = repo
        .create(&ct, json!("not an object"), None, &SaveContext::default())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn update_with_no_fields_returns_error() {
    let pool = setup_pool().await;
    let ct = parse_product();
    let repo = ContentRepository::new(pool);
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "X", "slug": "x", "price": 1})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

    let result = repo
        .update(
            &ct,
            raisfast::types::snowflake_id::SnowflakeId(id),
            json!({"nonexistent_field": "v"}),
            None,
            &SaveContext::default(),
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn migrate_adds_columns_incrementally() {
    let pool = setup_pool().await;
    let repo = ContentRepository::new(pool.clone());

    let mut ct_v1 = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_notes_v2"
implements = ["ownable", "timestampable"]

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    cache_ct(&mut ct_v1);
    repo.migrate(&ct_v1, &test_protocol_registry())
        .await
        .unwrap();

    let mut ct_v2 = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_notes_v2"
implements = ["ownable", "timestampable"]

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
    cache_ct(&mut ct_v2);
    repo.migrate(&ct_v2, &test_protocol_registry())
        .await
        .unwrap();

    let created = repo
        .create(
            &ct_v2,
            with_timestamps(json!({"title": "V2", "body": "hello", "priority": 5})),
            None,
            &SaveContext::default(),
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
implements = ["ownable", "timestampable", "versionable"]

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
    let mut ct = ContentTypeSchema::parse_from_str(VERSIONED_TOML).unwrap();
    cache_ct(&mut ct);
    ct
}

#[tokio::test]
async fn versioning_flag_parsed() {
    let ct = parse_versioned();
    assert!(ct.implements_protocol("versionable"));
    assert_eq!(ct.singular, "article");
}

#[tokio::test]
async fn versioning_creates_revision_on_update() {
    let pool = setup_pool().await;
    let ct = parse_versioned();
    let repo = ContentRepository::new(pool.clone());

    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "V1 Title", "content": "V1 Content"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let int_id: i64 = id.parse().unwrap();
    let _updated = repo
        .update(
            &ct,
            raisfast::types::snowflake_id::SnowflakeId(int_id),
            json!({"title": "V2 Title", "content": "V2 Content"}),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();

    let revisions = raisfast::models::content_revision::list_revisions(
        &pool,
        "article",
        SnowflakeId(id.parse::<i64>().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].revision_number, 1);

    let rev = raisfast::models::content_revision::get_revision(
        &pool,
        "article",
        SnowflakeId(id.parse::<i64>().unwrap()),
        SnowflakeId(*revisions[0].id),
    )
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

    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Rev0"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let int_id: i64 = id.parse().unwrap();
    repo.update(
        &ct,
        SnowflakeId(int_id),
        json!({"title": "Rev1"}),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();
    repo.update(
        &ct,
        SnowflakeId(int_id),
        json!({"title": "Rev2"}),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();
    repo.update(
        &ct,
        SnowflakeId(int_id),
        json!({"title": "Rev3"}),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();

    let revisions = raisfast::models::content_revision::list_revisions(
        &pool,
        "article",
        SnowflakeId(id.parse::<i64>().unwrap()),
    )
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

    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Temp"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let int_id: i64 = id.parse().unwrap();

    repo.update(
        &ct,
        SnowflakeId(int_id),
        json!({"title": "Updated"}),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();

    let before = raisfast::models::content_revision::list_revisions(
        &pool,
        "article",
        SnowflakeId(id.parse::<i64>().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(before.len(), 1);

    repo.delete(
        &ct,
        raisfast::types::snowflake_id::SnowflakeId(int_id),
        None,
        &test_protocol_registry(),
        &test_ct_registry(),
    )
    .await
    .unwrap();

    let after = raisfast::models::content_revision::list_revisions(
        &pool,
        "article",
        SnowflakeId(id.parse::<i64>().unwrap()),
    )
    .await
    .unwrap();
    assert!(after.is_empty());
}

#[tokio::test]
async fn versioning_no_revision_when_disabled() {
    let pool = setup_pool().await;
    let mut ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_no_versioning"
implements = ["ownable", "timestampable"]

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    cache_ct(&mut ct);
    assert!(!ct.implements_protocol("versionable"));

    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "NoRev"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let int_id: i64 = id.parse().unwrap();

    repo.update(
        &ct,
        SnowflakeId(int_id),
        json!({"title": "Updated"}),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();

    let revisions = raisfast::models::content_revision::list_revisions(
        &pool,
        "note",
        SnowflakeId(id.parse::<i64>().unwrap()),
    )
    .await
    .unwrap();
    assert!(revisions.is_empty());
}

#[tokio::test]
async fn versioning_diff_computes_correctly() {
    let old = json!({"title": "Old", "content": "Same", "status": "draft"});
    let new = json!({"title": "New", "content": "Same", "status": "published", "extra": 42});

    let diff = raisfast::models::content_revision::compute_diff(&old, &new);

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

#[tokio::test]
async fn soft_delete_filtered_from_list() {
    let pool = setup_pool().await;
    let mut ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_soft_filter_list"
implements = ["ownable", "timestampable", "soft_deletable"]

[fields.title]
type = "text"
required = true

[fields.slug]
type = "uid"
target_field = "title"
"#,
    )
    .unwrap();
    cache_ct(&mut ct);

    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    repo.create(
        &ct,
        with_timestamps(json!({"title": "Visible Note", "slug": "visible"})),
        None,
        &SaveContext::default(),
    )
    .await
    .unwrap();
    let deleted = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Deleted Note", "slug": "deleted"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let deleted_id: i64 = deleted["id"].as_str().unwrap().parse().unwrap();

    let now = now_str();
    repo.soft_delete(&ct, SnowflakeId(deleted_id), &now, None, None)
        .await
        .unwrap();

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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"].as_str().unwrap(), "Visible Note");
}

#[tokio::test]
async fn soft_delete_filtered_from_find_by_slug() {
    let pool = setup_pool().await;
    let mut ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_soft_filter_slug"
implements = ["ownable", "timestampable", "soft_deletable"]

[fields.title]
type = "text"
required = true

[fields.slug]
type = "uid"
target_field = "title"
"#,
    )
    .unwrap();
    cache_ct(&mut ct);

    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Gone", "slug": "gone"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let int_id: i64 = id.parse().unwrap();

    let now = now_str();
    repo.soft_delete(&ct, SnowflakeId(int_id), &now, None, None)
        .await
        .unwrap();

    let found = repo
        .find_by_slug(&ct, "gone", None, None, true)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn soft_delete_still_accessible_by_id() {
    let pool = setup_pool().await;
    let mut ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_soft_find_by_id"
implements = ["ownable", "timestampable", "soft_deletable"]

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    cache_ct(&mut ct);

    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Soft Deleted"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let int_id: i64 = id.parse().unwrap();

    let now = now_str();
    repo.soft_delete(&ct, SnowflakeId(int_id), &now, None, None)
        .await
        .unwrap();

    let found = repo
        .find_by_id(&ct, SnowflakeId(int_id), None, true)
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap()["title"].as_str().unwrap(), "Soft Deleted");
}

#[tokio::test]
async fn soft_delete_with_deleted_by() {
    let pool = setup_pool().await;
    let mut ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_soft_deleted_by"
implements = ["ownable", "timestampable", "soft_deletable"]

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    cache_ct(&mut ct);

    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Audit Me"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

    let now = now_str();
    repo.soft_delete(&ct, SnowflakeId(id), &now, Some(42), None)
        .await
        .unwrap();

    let row: (String, i64) =
        sqlx::query_as("SELECT deleted_at, deleted_by FROM ct_soft_deleted_by WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!row.0.is_empty());
    assert_eq!(row.1, 42);
}

#[tokio::test]
async fn soft_delete_implements_trait_equivalent() {
    let pool = setup_pool().await;
    let mut ct = ContentTypeSchema::parse_from_str(
        r#"
[content_type]
name = "Note"
singular = "note"
plural = "notes"
table = "ct_soft_via_implements"

implements = ["ownable", "timestampable", "soft_deletable"]

[fields.title]
type = "text"
required = true
"#,
    )
    .unwrap();
    cache_ct(&mut ct);

    assert!(ct.implements_protocol("soft_deletable"));

    let repo = ContentRepository::new(pool.clone());
    repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

    let created = repo
        .create(
            &ct,
            with_timestamps(json!({"title": "Implements Test"})),
            None,
            &SaveContext::default(),
        )
        .await
        .unwrap();
    let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

    let now = now_str();
    repo.soft_delete(&ct, SnowflakeId(id), &now, None, None)
        .await
        .unwrap();

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
        skip_total: false,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: false,
        meta_filters: Vec::new(),
    };
    let (items, total) = repo.find(&ct, query).await.unwrap();
    assert_eq!(total, 0);
    assert!(items.is_empty());
}

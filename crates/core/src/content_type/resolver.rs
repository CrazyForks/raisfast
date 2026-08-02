//! Content Type relation field resolution
//!
//! Expands relation fields after query, replacing FK IDs with full target records.
//! Similar to Strapi's "populate" mechanism.
//!
//! Supported relation types:
//! - `many_to_one` / `one_to_one` → embed a single record
//! - `one_to_many` → embed an array of records
//! - `many_to_many` → embed an array of records via junction table

use serde_json::{Value, json};
use sqlx::Row;

use super::schema::{ContentTypeSchema, FieldType, RelationType};
use crate::constants::COL_ID;

fn extract_id_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
use crate::db::DbDriver;
use crate::db::Pool;
use crate::errors::app_error::AppError;

/// Resolve relation fields in items (batch-optimized)
///
/// Batch queries by field dimension: collects FK IDs for the same relation field across all items,
/// fetches all target records with a single `WHERE id IN (?)` query, then distributes them back to each item by ID.
pub async fn resolve_relations(
    pool: &Pool,
    ct: &ContentTypeSchema,
    items: &mut [Value],
    include: Option<&[String]>,
) -> Result<(), AppError> {
    if items.is_empty() {
        return Ok(());
    }

    let include_set: Option<std::collections::HashSet<&str>> =
        include.map(|list| list.iter().map(|s| s.as_str()).collect());

    for field in &ct.fields {
        if field.field_type != FieldType::Relation {
            continue;
        }
        if let Some(set) = include_set.as_ref()
            && !set.contains(field.name.as_str())
        {
            continue;
        }
        let Some(ref rel) = field.relation else {
            continue;
        };

        match rel.relation_type {
            RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay => {
                resolve_many_to_one_batch(pool, ct, field, rel, items).await?;
            }
            RelationType::OneToMany => {
                resolve_one_to_many_batch(pool, ct, field.name.as_str(), rel, items).await?;
            }
            RelationType::ManyToMany | RelationType::ManyWay => {
                resolve_many_to_many_batch(pool, ct, field.name.as_str(), rel, items).await?;
            }
        }
    }
    Ok(())
}

async fn resolve_many_to_one_batch(
    pool: &Pool,
    _ct: &ContentTypeSchema,
    field: &super::schema::FieldSchema,
    rel: &super::schema::RelationConfig,
    items: &mut [Value],
) -> Result<(), AppError> {
    let fk = rel
        .foreign_key
        .clone()
        .unwrap_or_else(|| format!("{}_id", field.name));

    if !crate::db::driver::is_safe_identifier(&fk)
        || !crate::db::driver::is_safe_identifier(&rel.target)
    {
        tracing::warn!(
            "skipping many_to_one resolution with unsafe identifier: fk={fk}, target={}",
            rel.target
        );
        return Ok(());
    }

    let mut fk_ids: Vec<i64> = Vec::new();
    for item in &*items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(fk_val) = obj.get(&fk) else { continue };
        let Some(fk_id) = fk_val.as_i64() else {
            continue;
        };
        if fk_id > 0 {
            fk_ids.push(fk_id);
        }
    }
    if fk_ids.is_empty() {
        return Ok(());
    }

    let target_table = &rel.target;
    let columns = fetch_column_names(pool, target_table).await;
    let select_cols = columns.join(", ");

    let deduped_ids: Vec<i64> = {
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for id in &fk_ids {
            if seen.insert(*id) {
                deduped.push(*id);
            }
        }
        deduped
    };

    let placeholders: Vec<String> = (1..=deduped_ids.len()).map(crate::db::Driver::ph).collect();
    let sql = format!(
        "SELECT {select_cols} FROM {target_table} WHERE {COL_ID} IN ({})",
        placeholders.join(", ")
    );

    let mut q = sqlx::query(&sql);
    for id in &deduped_ids {
        q = q.bind(id);
    }
    let rows = q
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("batch relation query failed: {e}")))?;

    let id_cols: std::collections::HashSet<&str> = std::collections::HashSet::from([COL_ID]);
    let mut lookup: std::collections::HashMap<i64, Value> = std::collections::HashMap::new();
    for row in &rows {
        let val = super::repository::row_to_value(row, &columns, &id_cols);
        if let Some(id) = val.get(COL_ID).and_then(extract_id_i64) {
            lookup.insert(id, val);
        }
    }

    for item in items.iter_mut() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let Some(fk_val) = obj.get(&fk) else { continue };
        let Some(fk_id) = fk_val.as_i64() else {
            continue;
        };
        if fk_id == 0 {
            continue;
        }
        if let Some(target_data) = lookup.get(&fk_id) {
            obj.insert(field.name.clone(), target_data.clone());
        }
    }

    Ok(())
}

async fn resolve_one_to_many_batch(
    pool: &Pool,
    ct: &ContentTypeSchema,
    field_name: &str,
    rel: &super::schema::RelationConfig,
    items: &mut [Value],
) -> Result<(), AppError> {
    let fk_col = rel
        .foreign_key
        .clone()
        .unwrap_or_else(|| format!("{}_id", ct.singular));

    if !crate::db::driver::is_safe_identifier(&fk_col)
        || !crate::db::driver::is_safe_identifier(&rel.target)
    {
        tracing::warn!(
            "skipping one_to_many resolution with unsafe identifier: fk={fk_col}, target={}",
            rel.target
        );
        return Ok(());
    }

    let item_ids: Vec<i64> = items
        .iter()
        .filter_map(|item| item.get(COL_ID).and_then(extract_id_i64))
        .filter(|&id| id > 0)
        .collect();

    if item_ids.is_empty() {
        return Ok(());
    }

    let deduped_ids: Vec<i64> = {
        let mut seen = std::collections::HashSet::new();
        item_ids.into_iter().filter(|id| seen.insert(*id)).collect()
    };

    let target_table = &rel.target;
    let columns = fetch_column_names(pool, target_table).await;
    let select_cols = columns.join(", ");

    let placeholders: Vec<String> = (1..=deduped_ids.len()).map(crate::db::Driver::ph).collect();
    let sql = format!(
        "SELECT {select_cols}, {fk_col} as __fk FROM {target_table} WHERE {fk_col} IN ({})",
        placeholders.join(", ")
    );

    let mut q = sqlx::query(&sql);
    for id in &deduped_ids {
        q = q.bind(id);
    }
    let rows = q
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("batch one_to_many query failed: {e}")))?;

    let id_cols: std::collections::HashSet<&str> = std::collections::HashSet::from([COL_ID]);
    let mut lookup: std::collections::HashMap<i64, Vec<Value>> = std::collections::HashMap::new();
    for row in &rows {
        let fk_val: i64 = row.try_get("__fk").unwrap_or(0);
        let val = super::repository::row_to_value(row, &columns, &id_cols);
        lookup.entry(fk_val).or_default().push(val);
    }

    for item in items.iter_mut() {
        let Some(item_id) = item.get(COL_ID).and_then(extract_id_i64) else {
            continue;
        };
        let targets = lookup.get(&item_id).cloned().unwrap_or_default();
        if let Some(obj) = item.as_object_mut() {
            obj.insert(field_name.to_string(), json!(targets));
        }
    }

    Ok(())
}

async fn resolve_many_to_many_batch(
    pool: &Pool,
    ct: &ContentTypeSchema,
    field_name: &str,
    rel: &super::schema::RelationConfig,
    items: &mut [Value],
) -> Result<(), AppError> {
    let through = rel
        .through
        .clone()
        .unwrap_or_else(|| format!("{}_{}", ct.table, rel.target));

    let target_table = &rel.target;
    let source_col = format!("{}_id", ct.singular);
    let target_col = format!("{}_id", rel.target);

    if !crate::db::driver::is_safe_identifier(&through)
        || !crate::db::driver::is_safe_identifier(&source_col)
        || !crate::db::driver::is_safe_identifier(&target_col)
        || !crate::db::driver::is_safe_identifier(target_table)
    {
        tracing::warn!(
            "skipping many_to_many resolution with unsafe identifier: through={through}, source={source_col}, target={target_col}, table={target_table}"
        );
        return Ok(());
    }

    let item_ids: Vec<i64> = items
        .iter()
        .filter_map(|item| item.get(COL_ID).and_then(extract_id_i64))
        .filter(|&id| id > 0)
        .collect();

    if item_ids.is_empty() {
        return Ok(());
    }

    let deduped_ids: Vec<i64> = {
        let mut seen = std::collections::HashSet::new();
        item_ids.into_iter().filter(|id| seen.insert(*id)).collect()
    };

    let columns = fetch_column_names(pool, target_table).await;
    let select_cols = columns.join(", ");

    let placeholders: Vec<String> = (1..=deduped_ids.len()).map(crate::db::Driver::ph).collect();
    let sql = format!(
        "SELECT {select_cols}, {through}.{source_col} as __source_id \
         FROM {target_table} \
         INNER JOIN {through} ON {through}.{target_col} = {target_table}.{COL_ID} \
         WHERE {through}.{source_col} IN ({})",
        placeholders.join(", ")
    );

    let mut q = sqlx::query(&sql);
    for id in &deduped_ids {
        q = q.bind(id);
    }
    let rows = q
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("batch many_to_many query failed: {e}")))?;

    let id_cols: std::collections::HashSet<&str> = std::collections::HashSet::from([COL_ID]);
    let mut lookup: std::collections::HashMap<i64, Vec<Value>> = std::collections::HashMap::new();
    for row in &rows {
        let source_id: i64 = row.try_get("__source_id").unwrap_or(0);
        let val = super::repository::row_to_value(row, &columns, &id_cols);
        lookup.entry(source_id).or_default().push(val);
    }

    for item in items.iter_mut() {
        let Some(item_id) = item.get(COL_ID).and_then(extract_id_i64) else {
            continue;
        };
        let targets = lookup.get(&item_id).cloned().unwrap_or_default();
        if let Some(obj) = item.as_object_mut() {
            obj.insert(field_name.to_string(), json!(targets));
        }
    }

    Ok(())
}

async fn fetch_column_names(pool: &Pool, table: &str) -> Vec<String> {
    use std::sync::{LazyLock, RwLock};
    static CACHE: LazyLock<RwLock<std::collections::HashMap<String, Vec<String>>>> =
        LazyLock::new(|| RwLock::new(std::collections::HashMap::new()));

    {
        let cache = CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = cache.get(table) {
            return cached.clone();
        }
    }

    if !crate::db::driver::is_safe_identifier(table) {
        tracing::warn!(table, "rejected unsafe table name in fetch_column_names");
        return vec![COL_ID.into()];
    }

    let (sql, col_index) = match super::repository::fetch_columns_sql(table) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(table, "fetch_columns_sql error: {e}");
            return vec![COL_ID.into()];
        }
    };
    let cols: Vec<String> = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.iter()
                .map(|row| row.try_get(col_index).unwrap_or_default())
                .collect()
        })
        .unwrap_or_else(|_| vec![COL_ID.into()]);

    {
        let mut cache = CACHE.write().unwrap_or_else(|e| e.into_inner());
        cache.insert(table.to_string(), cols.clone());
    }

    cols
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_type::schema::ContentTypeSchema;

    fn make_ct_with_relations() -> ContentTypeSchema {
        ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "ct_resolve_posts"

[fields.title]
type = "text"
required = true

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "ct_resolve_users"
foreign_key = "author_id"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "ct_resolve_tags"
through = "ct_resolve_posts_tags"
"#,
        )
        .unwrap()
    }

    async fn setup_test_db() -> crate::db::Pool {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, slug TEXT, title TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, slug TEXT, title TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, author_id INTEGER, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, created_by INTEGER, updated_by INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_posts_tags (post_id INTEGER NOT NULL, ct_resolve_tags_id INTEGER NOT NULL, PRIMARY KEY (post_id, ct_resolve_tags_id))",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO ct_resolve_users (id, name, slug, title) VALUES (1, 'Alice', 'alice', '')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO ct_resolve_tags (id, name, slug, title) VALUES (1, 'Rust', 'rust', '')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO ct_resolve_tags (id, name, slug, title) VALUES (2, 'Web', 'web', '')",
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[tokio::test]
    async fn resolve_many_to_one() {
        let pool = setup_test_db().await;
        let ct = make_ct_with_relations();

        let mut items = vec![serde_json::json!({
            "id": 1,

            "title": "Hello",
            "author_id": 1,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        })];

        resolve_relations(&pool, &ct, &mut items, None)
            .await
            .unwrap();

        let author = items[0].get("author").unwrap();
        assert_eq!(author["name"], "Alice");
    }

    #[tokio::test]
    async fn resolve_many_to_many() {
        let pool = setup_test_db().await;
        let ct = make_ct_with_relations();

        sqlx::query(
            "INSERT INTO ct_resolve_posts (id, title, author_id, created_at, updated_at) VALUES (1, 'Hello', 1, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO ct_resolve_posts_tags (post_id, ct_resolve_tags_id) VALUES (1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ct_resolve_posts_tags (post_id, ct_resolve_tags_id) VALUES (1, 2)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut items = vec![serde_json::json!({
            "id": 1,

            "title": "Hello",
            "author_id": 1,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        })];

        resolve_relations(&pool, &ct, &mut items, None)
            .await
            .unwrap();

        let tags = items[0].get("tags").unwrap().as_array().unwrap();
        assert_eq!(tags.len(), 2);
        let names: Vec<&str> = tags.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"Rust"));
        assert!(names.contains(&"Web"));
    }

    #[tokio::test]
    async fn resolve_with_include_filter() {
        let pool = setup_test_db().await;
        let ct = make_ct_with_relations();

        let mut items = vec![serde_json::json!({
            "id": 1,

            "title": "Hello",
            "author_id": 1,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        })];

        let include = vec!["author".to_string()];
        resolve_relations(&pool, &ct, &mut items, Some(&include))
            .await
            .unwrap();

        assert!(items[0].get("author").is_some());
        assert!(items[0].get("tags").is_none());
    }

    #[tokio::test]
    async fn resolve_empty_items() {
        let pool = setup_test_db().await;
        let ct = make_ct_with_relations();
        let mut items: Vec<Value> = vec![];
        let result = resolve_relations(&pool, &ct, &mut items, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn resolve_one_to_many() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT, post_id INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, author_id INTEGER, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO ct_resolve_posts (id, title, author_id, created_at, updated_at) VALUES (1, 'Hello', 0, '2024-01-01', '2024-01-01')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO ct_resolve_comments (id, text, post_id) VALUES (1, 'Nice', 1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ct_resolve_comments (id, text, post_id) VALUES (2, 'Great', 1)")
            .execute(&pool)
            .await
            .unwrap();

        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "ct_resolve_posts"

[fields.title]
type = "text"

[fields.comments]
type = "relation"
relation_type = "one_to_many"
target = "ct_resolve_comments"
foreign_key = "post_id"
"#,
        )
        .unwrap();

        let mut items = vec![serde_json::json!({
            "id": 1,

            "title": "Hello",
            "created_at": "2024-01-01",
            "updated_at": "2024-01-01"
        })];

        resolve_relations(&pool, &ct, &mut items, None)
            .await
            .unwrap();

        let comments = items[0].get("comments").unwrap().as_array().unwrap();
        assert_eq!(comments.len(), 2);
        let texts: Vec<&str> = comments.iter().filter_map(|c| c["text"].as_str()).collect();
        assert!(texts.contains(&"Nice"));
        assert!(texts.contains(&"Great"));
    }

    #[tokio::test]
    async fn resolve_m2o_with_zero_fk_skipped() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();

        sqlx::query("CREATE TABLE ct_resolve_users (id INTEGER PRIMARY KEY, name TEXT)")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_posts (id INTEGER PRIMARY KEY, title TEXT, author_id INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "ct_resolve_posts"

[fields.title]
type = "text"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "ct_resolve_users"
foreign_key = "author_id"
"#,
        )
        .unwrap();

        let mut items = vec![serde_json::json!({
            "id": 1,

            "title": "NoAuthor",
            "author_id": 0
        })];

        resolve_relations(&pool, &ct, &mut items, None)
            .await
            .unwrap();

        assert!(items[0].get("author").is_none());
    }
}

//! Content Type 关系字段解析
//!
//! 查询后展开 relation 字段，将 FK ID 替换为目标记录的完整数据。
//! 类似 Strapi 的 "populate" 机制。
//!
//! 支持的关系类型：
//! - `many_to_one` / `one_to_one` → 嵌入单条记录
//! - `one_to_many` → 嵌入记录数组
//! - `many_to_many` → 通过 junction 表嵌入记录数组

use serde_json::{Value, json};
use sqlx::Row;

use super::schema::{ContentTypeSchema, FieldType, RelationType};
use crate::db::Pool;
use crate::errors::app_error::AppError;

/// 解析 items 中的 relation 字段（批量优化）
///
/// 按字段维度批量查询：收集所有 items 中同一 relation 字段的 FK ID，
/// 一次 `WHERE id IN (?)` 查询获取全部目标记录，再按 ID 分发回各 item。
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
            RelationType::ManyToOne
            | RelationType::OneToOne
            | RelationType::OneWay
            | RelationType::ManyWay => {
                resolve_many_to_one_batch(pool, ct, field, rel, items).await?;
            }
            RelationType::OneToMany => {
                resolve_one_to_many_batch(pool, ct, field.name.as_str(), rel, items).await?;
            }
            RelationType::ManyToMany => {
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

    let mut fk_ids: Vec<String> = Vec::new();
    for item in &*items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(fk_val) = obj.get(&fk) else { continue };
        let Some(fk_id) = fk_val.as_str() else {
            continue;
        };
        if !fk_id.is_empty() {
            fk_ids.push(fk_id.to_string());
        }
    }
    if fk_ids.is_empty() {
        return Ok(());
    }

    let target_table = &rel.target;
    let columns = fetch_column_names(pool, target_table).await;
    let select_cols = columns.join(", ");

    let deduped_ids: Vec<String> = {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for id in &fk_ids {
            if seen.insert(id.clone()) {
                deduped.push(id.clone());
            }
        }
        deduped
    };

    let placeholders: Vec<String> = (1..=deduped_ids.len())
        .map(crate::db::dialect::ph)
        .collect();
    let sql = format!(
        "SELECT {select_cols} FROM {target_table} WHERE id IN ({})",
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

    let mut lookup: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for row in &rows {
        let val = super::repository::row_to_value(row, &columns);
        if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
            lookup.insert(id.to_string(), val);
        }
    }

    for item in items.iter_mut() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let Some(fk_val) = obj.get(&fk) else { continue };
        let Some(fk_id) = fk_val.as_str() else {
            continue;
        };
        if fk_id.is_empty() {
            continue;
        }
        if let Some(target_data) = lookup.get(fk_id) {
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

    let item_ids: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .collect();

    if item_ids.is_empty() {
        return Ok(());
    }

    let deduped_ids: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        item_ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect()
    };

    let target_table = &rel.target;
    let columns = fetch_column_names(pool, target_table).await;
    let select_cols = columns.join(", ");

    let placeholders: Vec<String> = (1..=deduped_ids.len())
        .map(crate::db::dialect::ph)
        .collect();
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

    let mut lookup: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for row in &rows {
        let fk_val: String = row.try_get("__fk").unwrap_or_default();
        let val = super::repository::row_to_value(row, &columns);
        lookup.entry(fk_val).or_default().push(val);
    }

    for item in items.iter_mut() {
        let Some(item_id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let targets = lookup.get(item_id).cloned().unwrap_or_default();
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

    let item_ids: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .collect();

    if item_ids.is_empty() {
        return Ok(());
    }

    let deduped_ids: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        item_ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect()
    };

    let target_table = &rel.target;
    let source_col = format!("{}_id", ct.singular);
    let target_col = format!("{}_id", rel.target);
    let columns = fetch_column_names(pool, target_table).await;
    let select_cols = columns.join(", ");

    let placeholders: Vec<String> = (1..=deduped_ids.len())
        .map(crate::db::dialect::ph)
        .collect();
    let sql = format!(
        "SELECT {select_cols}, {through}.{source_col} as __source_id \
         FROM {target_table} \
         INNER JOIN {through} ON {through}.{target_col} = {target_table}.id \
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

    let mut lookup: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for row in &rows {
        let source_id: String = row.try_get("__source_id").unwrap_or_default();
        let val = super::repository::row_to_value(row, &columns);
        lookup.entry(source_id).or_default().push(val);
    }

    for item in items.iter_mut() {
        let Some(item_id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let targets = lookup.get(item_id).cloned().unwrap_or_default();
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

    if !crate::db::dialect::is_safe_identifier(table) {
        tracing::warn!(table, "rejected unsafe table name in fetch_column_names");
        return vec!["id".into()];
    }

    let (sql, col_index) = super::repository::fetch_columns_sql(table);
    let cols: Vec<String> = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.iter()
                .map(|row| row.try_get(col_index).unwrap_or_default())
                .collect()
        })
        .unwrap_or_else(|_| vec!["id".into()]);

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
            "CREATE TABLE ct_resolve_users (id TEXT PRIMARY KEY, name TEXT, slug TEXT, title TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_tags (id TEXT PRIMARY KEY, name TEXT, slug TEXT, title TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_posts (id TEXT PRIMARY KEY, title TEXT, author_id TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, created_by TEXT, updated_by TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE ct_resolve_posts_tags (post_id TEXT NOT NULL, ct_resolve_tags_id TEXT NOT NULL, PRIMARY KEY (post_id, ct_resolve_tags_id))",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO ct_resolve_users (id, name, slug, title) VALUES ('u1', 'Alice', 'alice', '')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO ct_resolve_tags (id, name, slug, title) VALUES ('t1', 'Rust', 'rust', '')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO ct_resolve_tags (id, name, slug, title) VALUES ('t2', 'Web', 'web', '')",
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
            "id": "p1",
            "title": "Hello",
            "author_id": "u1",
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
            "INSERT INTO ct_resolve_posts_tags (post_id, ct_resolve_tags_id) VALUES ('p1', 't1')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ct_resolve_posts_tags (post_id, ct_resolve_tags_id) VALUES ('p1', 't2')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut items = vec![serde_json::json!({
            "id": "p1",
            "title": "Hello",
            "author_id": "u1",
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
            "id": "p1",
            "title": "Hello",
            "author_id": "u1",
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
}

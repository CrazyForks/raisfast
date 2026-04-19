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

use super::schema::{ContentTypeSchema, FieldType, RelationType};
use crate::errors::app_error::AppError;

/// 解析 items 中的 relation 字段
///
/// 对每个 item，检查 schema 中的 relation 字段：
/// - 如果该字段名在 `include` 列表中（或 include 为空表示全部展开），
///   则用目标记录的 JSON 替换 FK ID 值。
pub async fn resolve_relations(
    pool: &sqlx::SqlitePool,
    ct: &ContentTypeSchema,
    items: &mut [Value],
    include: Option<&[String]>,
) -> Result<(), AppError> {
    let include_set: Option<std::collections::HashSet<&str>> =
        include.map(|list| list.iter().map(|s| s.as_str()).collect());

    for item in items {
        resolve_item_relations(pool, ct, item, include_set.as_ref()).await?;
    }
    Ok(())
}

async fn resolve_item_relations(
    pool: &sqlx::SqlitePool,
    ct: &ContentTypeSchema,
    item: &mut Value,
    include_set: Option<&std::collections::HashSet<&str>>,
) -> Result<(), AppError> {
    let Some(obj) = item.as_object_mut() else {
        return Ok(());
    };

    for field in &ct.fields {
        if field.field_type != FieldType::Relation {
            continue;
        }

        if let Some(set) = include_set
            && !set.contains(field.name.as_str())
        {
            continue;
        }

        let Some(ref rel) = field.relation else {
            continue;
        };

        match rel.relation_type {
            RelationType::ManyToOne | RelationType::OneToOne => {
                let fk = rel
                    .foreign_key
                    .clone()
                    .unwrap_or_else(|| format!("{}_id", field.name));

                let Some(fk_val) = obj.get(&fk) else {
                    continue;
                };
                let Some(fk_id) = fk_val.as_str() else {
                    continue;
                };
                if fk_id.is_empty() {
                    continue;
                }

                let target_table = &rel.target;
                let cols = build_star_columns(pool, target_table).await;
                let sql = format!("SELECT json_object({cols}) FROM {target_table} WHERE id = ?");
                let sql = crate::db::dialect::translate(&sql);

                let row = sqlx::query_as::<_, (Option<String>,)>(&sql)
                    .bind(fk_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| {
                        AppError::Internal(anyhow::anyhow!("relation query failed: {e}"))
                    })?;

                if let Some((Some(json_str),)) = row
                    && let Ok(target_data) = serde_json::from_str::<Value>(&json_str)
                {
                    obj.insert(field.name.clone(), target_data);
                }
            }
            RelationType::OneToMany => {
                let fk_col = rel
                    .foreign_key
                    .clone()
                    .unwrap_or_else(|| format!("{}_id", ct.singular));

                let item_id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if item_id.is_empty() {
                    continue;
                }

                let target_table = &rel.target;
                let cols = build_star_columns(pool, target_table).await;
                let sql =
                    format!("SELECT json_object({cols}) FROM {target_table} WHERE {fk_col} = ?");
                let sql = crate::db::dialect::translate(&sql);

                let rows = sqlx::query_as::<_, (String,)>(&sql)
                    .bind(item_id)
                    .fetch_all(pool)
                    .await
                    .map_err(|e| {
                        AppError::Internal(anyhow::anyhow!("relation query failed: {e}"))
                    })?;

                let targets: Vec<Value> = rows
                    .into_iter()
                    .filter_map(|(s,)| serde_json::from_str::<Value>(&s).ok())
                    .collect();

                obj.insert(field.name.clone(), json!(targets));
            }
            RelationType::ManyToMany => {
                let through = rel
                    .through
                    .clone()
                    .unwrap_or_else(|| format!("{}_{}", ct.table, rel.target));

                let item_id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if item_id.is_empty() {
                    continue;
                }

                let target_table = &rel.target;
                let source_col = format!("{}_id", ct.singular);
                let target_col = format!("{}_id", rel.target);
                let cols = build_star_columns(pool, target_table).await;

                let sql = format!(
                    "SELECT json_object({cols}) FROM {target_table} \
                     INNER JOIN {through} ON {through}.{target_col} = {target_table}.id \
                     WHERE {through}.{source_col} = ?"
                );
                let sql = crate::db::dialect::translate(&sql);

                let rows = sqlx::query_as::<_, (String,)>(&sql)
                    .bind(item_id)
                    .fetch_all(pool)
                    .await
                    .map_err(|e| {
                        AppError::Internal(anyhow::anyhow!("relation query failed: {e}"))
                    })?;

                let targets: Vec<Value> = rows
                    .into_iter()
                    .filter_map(|(s,)| serde_json::from_str::<Value>(&s).ok())
                    .collect();

                obj.insert(field.name.clone(), json!(targets));
            }
            RelationType::OneWay | RelationType::ManyWay => {
                let fk = rel
                    .foreign_key
                    .clone()
                    .unwrap_or_else(|| format!("{}_id", field.name));

                let Some(fk_val) = obj.get(&fk) else {
                    continue;
                };
                let Some(fk_id) = fk_val.as_str() else {
                    continue;
                };
                if fk_id.is_empty() {
                    continue;
                }

                let target_table = &rel.target;
                let cols = build_star_columns(pool, target_table).await;
                let sql = format!("SELECT json_object({cols}) FROM {target_table} WHERE id = ?");
                let sql = crate::db::dialect::translate(&sql);

                let row = sqlx::query_as::<_, (Option<String>,)>(&sql)
                    .bind(fk_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| {
                        AppError::Internal(anyhow::anyhow!("relation query failed: {e}"))
                    })?;

                if let Some((Some(json_str),)) = row
                    && let Ok(target_data) = serde_json::from_str::<Value>(&json_str)
                {
                    obj.insert(field.name.clone(), target_data);
                }
            }
        }
    }

    Ok(())
}

async fn build_star_columns(pool: &sqlx::SqlitePool, table: &str) -> String {
    let cols: Vec<String> = sqlx::query_as::<_, (String,)>(&format!(
        "SELECT name FROM pragma_table_info('{}') ORDER BY cid",
        table
    ))
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|(c,)| c).collect())
    .unwrap_or_else(|_| vec!["id".into()]);

    let mut parts = Vec::new();
    for col in &cols {
        parts.push(format!("'{}', {}", col, col));
    }
    parts.join(", ")
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
timestamps = true

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

    async fn setup_test_db() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();

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
            "CREATE TABLE ct_resolve_posts (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default', title TEXT, author_id TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
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

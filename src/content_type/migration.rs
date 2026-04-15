//! Schema → SQL Migration 生成器
//!
//! 根据 `ContentTypeSchema` 定义自动生成 CREATE TABLE / ALTER TABLE SQL。
//! 使用 `crate::db::dialect::translate()` 支持多数据库方言。

use super::schema::{ContentTypeSchema, FieldType, RelationType};

/// 根据内容类型定义生成 CREATE TABLE SQL
#[must_use]
pub fn generate_create_table(ct: &ContentTypeSchema) -> String {
    let mut cols = Vec::new();

    cols.push("    id TEXT PRIMARY KEY".to_string());
    cols.push("    tenant_id TEXT NOT NULL DEFAULT 'default'".to_string());

    for field in &ct.fields {
        if field.field_type == FieldType::Relation {
            match field.relation.as_ref().map(|r| &r.relation_type) {
                Some(RelationType::ManyToOne | RelationType::OneToOne) => {
                    let fk = field
                        .relation
                        .as_ref()
                        .and_then(|r| r.foreign_key.clone())
                        .unwrap_or_else(|| format!("{}_id", field.name));
                    let target_table = field
                        .relation
                        .as_ref()
                        .map_or("users", |r| r.target.as_str());
                    let not_null = if field.required { " NOT NULL" } else { "" };
                    cols.push(format!(
                        "    {fk} TEXT{not_null} REFERENCES {target_table}(id)"
                    ));
                }
                Some(RelationType::ManyToMany) => {
                    // junction table 后续单独生成
                }
                _ => {}
            }
            continue;
        }

        let col_type = field_type_to_sql(&field.field_type);
        let mut col_def = format!("    {} {}", field.name, col_type);

        if field.required && field.default.is_none() && field.field_type != FieldType::Boolean {
            col_def.push_str(" NOT NULL");
        }

        if let Some(ref default) = field.default {
            col_def.push_str(&format!(" DEFAULT {}", json_to_sql_literal(default)));
        }

        cols.push(col_def);
    }

    if ct.draft_publish {
        cols.push("    status TEXT NOT NULL DEFAULT 'draft'".to_string());
        cols.push("    published_at TEXT".to_string());
    }

    if ct.timestamps {
        cols.push("    created_at TEXT NOT NULL".to_string());
        cols.push("    updated_at TEXT NOT NULL".to_string());
    }

    if ct.soft_delete {
        cols.push("    deleted_at TEXT".to_string());
    }

    let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (\n", ct.table);
    sql.push_str(&cols.join(",\n"));
    sql.push_str("\n)");

    sql
}

/// 生成多对多 junction 表的 CREATE TABLE SQL
#[must_use]
pub fn generate_junction_tables(ct: &ContentTypeSchema) -> Vec<String> {
    let mut tables = Vec::new();

    for field in &ct.fields {
        if let Some(ref rel) = field.relation
            && rel.relation_type == RelationType::ManyToMany
        {
            let through = rel
                .through
                .clone()
                .unwrap_or_else(|| format!("{}_{}", ct.table, rel.target));
            let target_table = &rel.target;
            let source_col = format!("{}_id", ct.singular);
            let target_col = format!("{}_id", rel.target);

            let sql = format!(
                "CREATE TABLE IF NOT EXISTS {through} (\n\
                 {source_col} TEXT NOT NULL REFERENCES {source_table}(id) ON DELETE CASCADE,\n\
                 {target_col} TEXT NOT NULL REFERENCES {target_table}(id) ON DELETE CASCADE,\n\
                 PRIMARY KEY ({source_col}, {target_col})\n\
                 )",
                through = through,
                source_col = source_col,
                source_table = ct.table,
                target_col = target_col,
                target_table = target_table,
            );
            tables.push(sql);
        }
    }

    tables
}

/// 生成索引 CREATE INDEX SQL
#[must_use]
pub fn generate_indexes(ct: &ContentTypeSchema) -> Vec<String> {
    let mut indexes = Vec::new();

    for field in &ct.fields {
        if field.unique && field.field_type != FieldType::Uid {
            let idx_name = format!("idx_{}_{}_unique", ct.table, field.name);
            indexes.push(format!(
                "CREATE UNIQUE INDEX IF NOT EXISTS {idx_name} ON {}({})",
                ct.table, field.name
            ));
        }
    }

    for idx in &ct.indexes {
        let cols = idx.fields.join(",");
        if idx.unique {
            let idx_name = format!("idx_{}_{}_unique", ct.table, idx.fields.join("_"));
            indexes.push(format!(
                "CREATE UNIQUE INDEX IF NOT EXISTS {idx_name} ON {}({})",
                ct.table, cols
            ));
        } else {
            let idx_name = format!("idx_{}_{}", ct.table, idx.fields.join("_"));
            indexes.push(format!(
                "CREATE INDEX IF NOT EXISTS {idx_name} ON {}({})",
                ct.table, cols
            ));
        }
    }

    if ct.draft_publish {
        let idx_name = format!("idx_{}_status_created", ct.table);
        indexes.push(format!(
            "CREATE INDEX IF NOT EXISTS {idx_name} ON {}(status, created_at)",
            ct.table
        ));
    }

    indexes
}

/// 根据内容类型定义和已有列，生成 ALTER TABLE ADD COLUMN SQL
///
/// 对比 schema 期望的列与数据库中已有的列，只为缺失的列生成 DDL。
/// 不删除列、不修改列类型——只做增量添加（与 Strapi `forceMigration` 策略一致）。
#[must_use]
pub fn generate_alter_table(ct: &ContentTypeSchema, existing_columns: &[String]) -> Vec<String> {
    let existing: std::collections::HashSet<&str> = existing_columns
        .iter()
        .map(std::string::String::as_str)
        .collect();
    let mut stmts = Vec::new();

    // id 列不应该缺失（表已存在说明有主键），跳过

    for field in &ct.fields {
        if field.field_type == FieldType::Relation {
            match field.relation.as_ref().map(|r| &r.relation_type) {
                Some(RelationType::ManyToOne | RelationType::OneToOne) => {
                    let fk = field
                        .relation
                        .as_ref()
                        .and_then(|r| r.foreign_key.clone())
                        .unwrap_or_else(|| format!("{}_id", field.name));
                    let target_table = field
                        .relation
                        .as_ref()
                        .map_or("users", |r| r.target.as_str());
                    if !existing.contains(fk.as_str()) {
                        let not_null = if field.required { " NOT NULL" } else { "" };
                        stmts.push(format!(
                            "ALTER TABLE {} ADD COLUMN {fk} TEXT{not_null} REFERENCES {target_table}(id)",
                            ct.table
                        ));
                    }
                }
                Some(RelationType::ManyToMany) => {}
                _ => {}
            }
            continue;
        }

        if !existing.contains(field.name.as_str()) {
            let col_type = field_type_to_sql(&field.field_type);
            let mut sql = format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                ct.table, field.name, col_type
            );

            if field.required && field.default.is_none() && field.field_type != FieldType::Boolean {
                sql.push_str(" NOT NULL");
            }

            if let Some(ref default) = field.default {
                sql.push_str(&format!(" DEFAULT {}", json_to_sql_literal(default)));
            }

            stmts.push(sql);
        }
    }

    if ct.draft_publish {
        if !existing.contains("status") {
            stmts.push(format!(
                "ALTER TABLE {} ADD COLUMN status TEXT NOT NULL DEFAULT 'draft'",
                ct.table
            ));
        }
        if !existing.contains("published_at") {
            stmts.push(format!(
                "ALTER TABLE {} ADD COLUMN published_at TEXT",
                ct.table
            ));
        }
    }

    if ct.timestamps {
        if !existing.contains("created_at") {
            stmts.push(format!(
                "ALTER TABLE {} ADD COLUMN created_at TEXT NOT NULL",
                ct.table
            ));
        }
        if !existing.contains("updated_at") {
            stmts.push(format!(
                "ALTER TABLE {} ADD COLUMN updated_at TEXT NOT NULL",
                ct.table
            ));
        }
    }

    if ct.soft_delete && !existing.contains("deleted_at") {
        stmts.push(format!(
            "ALTER TABLE {} ADD COLUMN deleted_at TEXT",
            ct.table
        ));
    }

    stmts
}

/// 获取 content type schema 期望的所有列名（用于与 DB 对比）
#[must_use]
pub fn expected_columns(ct: &ContentTypeSchema) -> Vec<String> {
    let mut cols = vec!["id".to_string()];

    for field in &ct.fields {
        if field.field_type == FieldType::Relation {
            if let Some(RelationType::ManyToOne | RelationType::OneToOne) =
                field.relation.as_ref().map(|r| &r.relation_type)
            {
                let fk = field
                    .relation
                    .as_ref()
                    .and_then(|r| r.foreign_key.clone())
                    .unwrap_or_else(|| format!("{}_id", field.name));
                cols.push(fk);
            }
            continue;
        }
        cols.push(field.name.clone());
    }

    if ct.draft_publish {
        cols.push("status".into());
        cols.push("published_at".into());
    }
    if ct.timestamps {
        cols.push("created_at".into());
        cols.push("updated_at".into());
    }
    if ct.soft_delete {
        cols.push("deleted_at".into());
    }

    cols
}

fn field_type_to_sql(ft: &FieldType) -> &'static str {
    match ft {
        FieldType::Text | FieldType::RichText | FieldType::Json => "TEXT",
        FieldType::Integer | FieldType::BigInt => "INTEGER",
        FieldType::Decimal | FieldType::Float => "REAL",
        FieldType::Boolean => "BOOLEAN",
        FieldType::Date | FieldType::DateTime | FieldType::Time => "TEXT",
        FieldType::Email | FieldType::Password | FieldType::Enum => "TEXT",
        FieldType::Uid => "TEXT",
        FieldType::Media => "TEXT",
        FieldType::Relation => "TEXT",
    }
}

fn json_to_sql_literal(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                "1".into()
            } else {
                "0".into()
            }
        }
        serde_json::Value::Null => "NULL".into(),
        other => format!("'{}'", other.to_string().replace('\'', "''")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_post_ct() -> ContentTypeSchema {
        ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
draft_publish = true
slug_field = "title"
timestamps = true

[fields.title]
type = "text"
required = true
max_length = 200

[fields.slug]
type = "uid"
unique = true

[fields.content]
type = "richtext"
required = true

[fields.status]
type = "enum"
values = ["draft", "published", "archived"]
default = "draft"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
through = "posts_tags"

[fields.view_count]
type = "integer"
default = 0
private = true

[fields.is_pinned]
type = "boolean"
default = false

[[indexes]]
fields = ["slug"]
unique = true
"#,
        )
        .unwrap()
    }

    #[test]
    fn generate_create_table_sql() {
        let ct = make_post_ct();
        let sql = generate_create_table(&ct);

        assert!(sql.contains("CREATE TABLE IF NOT EXISTS posts"));
        assert!(sql.contains("id TEXT PRIMARY KEY"));
        assert!(sql.contains("title TEXT NOT NULL"));
        assert!(sql.contains("content TEXT NOT NULL"));
        assert!(sql.contains("author_id TEXT REFERENCES users(id)"));
        assert!(sql.contains("status TEXT NOT NULL DEFAULT 'draft'"));
        assert!(sql.contains("published_at TEXT"));
        assert!(sql.contains("created_at TEXT NOT NULL"));
        assert!(sql.contains("updated_at TEXT NOT NULL"));
        assert!(sql.contains("view_count INTEGER DEFAULT 0"));
        assert!(sql.contains("is_pinned BOOLEAN DEFAULT 0"));
        assert!(sql.contains("slug TEXT"));
    }

    #[test]
    fn generate_junction_tables_sql() {
        let ct = make_post_ct();
        let junctions = generate_junction_tables(&ct);
        assert_eq!(junctions.len(), 1);
        assert!(junctions[0].contains("CREATE TABLE IF NOT EXISTS posts_tags"));
        assert!(junctions[0].contains("post_id TEXT NOT NULL REFERENCES posts(id)"));
        assert!(junctions[0].contains("tags_id TEXT NOT NULL REFERENCES tags(id)"));
        assert!(junctions[0].contains("PRIMARY KEY (post_id, tags_id)"));
    }

    #[test]
    fn generate_indexes_sql() {
        let ct = make_post_ct();
        let indexes = generate_indexes(&ct);
        assert!(!indexes.is_empty());
        assert!(indexes.iter().any(|i| i.contains("idx_posts_slug_unique")));
        assert!(
            indexes
                .iter()
                .any(|i| i.contains("idx_posts_status_created"))
        );
    }

    #[test]
    fn generate_simple_table() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Tag"
singular = "tag"
plural = "tags"
table = "tags"
timestamps = true

[fields.name]
type = "text"
required = true

[fields.slug]
type = "uid"
unique = true
"#,
        )
        .unwrap();

        let sql = generate_create_table(&ct);
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS tags"));
        assert!(sql.contains("name TEXT NOT NULL"));
        assert!(sql.contains("created_at TEXT NOT NULL"));
        assert!(!sql.contains("status"));
    }

    #[test]
    fn generate_soft_delete_table() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
timestamps = true
soft_delete = true

[fields.name]
type = "text"
required = true
"#,
        )
        .unwrap();

        let sql = generate_create_table(&ct);
        assert!(sql.contains("deleted_at TEXT"));
    }

    #[test]
    fn alter_table_adds_missing_columns() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
draft_publish = true
timestamps = true

[fields.title]
type = "text"
required = true

[fields.slug]
type = "uid"
unique = true

[fields.content]
type = "richtext"
required = true

[fields.view_count]
type = "integer"
default = 0
private = true

[fields.is_pinned]
type = "boolean"
default = false
"#,
        )
        .unwrap();

        // 模拟 DB 已有 id, title, slug, content, status, published_at, created_at, updated_at
        let existing = vec![
            "id".into(),
            "title".into(),
            "slug".into(),
            "content".into(),
            "status".into(),
            "published_at".into(),
            "created_at".into(),
            "updated_at".into(),
        ];

        let stmts = generate_alter_table(&ct, &existing);
        assert_eq!(stmts.len(), 2);
        assert!(
            stmts
                .iter()
                .any(|s| s.contains("view_count INTEGER DEFAULT 0"))
        );
        assert!(
            stmts
                .iter()
                .any(|s| s.contains("is_pinned BOOLEAN DEFAULT 0"))
        );
    }

    #[test]
    fn alter_table_nothing_when_up_to_date() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Tag"
singular = "tag"
plural = "tags"
table = "tags"
timestamps = true

[fields.name]
type = "text"
required = true

[fields.slug]
type = "uid"
unique = true
"#,
        )
        .unwrap();

        let existing = vec![
            "id".into(),
            "name".into(),
            "slug".into(),
            "created_at".into(),
            "updated_at".into(),
        ];

        let stmts = generate_alter_table(&ct, &existing);
        assert!(stmts.is_empty());
    }

    #[test]
    fn alter_table_adds_system_columns() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
draft_publish = true
timestamps = true
soft_delete = true

[fields.title]
type = "text"
required = true
"#,
        )
        .unwrap();

        // 只有 id 和 title（缺少所有系统列）
        let existing = vec!["id".into(), "title".into()];

        let stmts = generate_alter_table(&ct, &existing);
        assert!(
            stmts
                .iter()
                .any(|s| s.contains("status TEXT NOT NULL DEFAULT 'draft'"))
        );
        assert!(stmts.iter().any(|s| s.contains("published_at TEXT")));
        assert!(stmts.iter().any(|s| s.contains("created_at TEXT NOT NULL")));
        assert!(stmts.iter().any(|s| s.contains("updated_at TEXT NOT NULL")));
        assert!(stmts.iter().any(|s| s.contains("deleted_at TEXT")));
    }

    #[test]
    fn expected_columns_matches_schema() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
draft_publish = true
timestamps = true

[fields.title]
type = "text"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
foreign_key = "author_id"
"#,
        )
        .unwrap();

        let cols = expected_columns(&ct);
        assert!(cols.contains(&"id".to_string()));
        assert!(cols.contains(&"title".to_string()));
        assert!(cols.contains(&"author_id".to_string()));
        assert!(cols.contains(&"status".to_string()));
        assert!(cols.contains(&"created_at".to_string()));
    }
}

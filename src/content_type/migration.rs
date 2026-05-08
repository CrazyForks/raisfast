//! Schema → SQL Migration 生成器
//!
//! 根据 `ContentTypeSchema` 定义自动生成 CREATE TABLE / ALTER TABLE SQL。

use super::schema::{ContentTypeSchema, FieldType, RelationType};

use crate::aspects::ColumnDef;

/// 根据内容类型定义生成 CREATE TABLE SQL
///
/// `protocol_columns` 从 `ProtocolRegistry::columns_for()` 获取，
/// 包含所有 implements 协议声明的列（如 ownable、timestampable、soft_deletable）。
#[must_use]
pub fn generate_create_table(ct: &ContentTypeSchema, protocol_columns: &[ColumnDef]) -> String {
    let mut cols = Vec::new();

    cols.push("    id INTEGER PRIMARY KEY AUTOINCREMENT".to_string());
    cols.push("    document_id TEXT NOT NULL UNIQUE".to_string());

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
                        "    {fk} INTEGER{not_null} REFERENCES {target_table}(id)"
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

    let user_col_names: std::collections::HashSet<&str> = ct
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .chain(
            ct.fields
                .iter()
                .filter(|f| f.field_type == FieldType::Relation)
                .filter_map(|f| {
                    f.relation
                        .as_ref()
                        .and_then(|r| r.foreign_key.as_deref())
                        .or_else(|| Some(f.name.as_str()).filter(|n| n.ends_with("_id")))
                }),
        )
        .collect();

    for col in protocol_columns {
        if !user_col_names.contains(col.name.as_str()) {
            let mut sql = format!("    {} {}", col.name, col.sql_type.as_str());
            if let Some(ref default) = col.default {
                sql.push_str(&format!(" NOT NULL DEFAULT {}", default));
            }
            cols.push(sql);
        }
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
                 {source_col} INTEGER NOT NULL REFERENCES {source_table}(id) ON DELETE CASCADE,\n\
                 {target_col} INTEGER NOT NULL REFERENCES {target_table}(id) ON DELETE CASCADE,\n\
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

    indexes
}

/// 根据内容类型定义和已有列，生成 ALTER TABLE ADD COLUMN SQL
///
/// 对比 schema 期望的列与数据库中已有的列，只为缺失的列生成 DDL。
/// 不删除列、不修改列类型——只做增量添加（与 Strapi `forceMigration` 策略一致）。
#[must_use]
pub fn generate_alter_table(
    ct: &ContentTypeSchema,
    existing_columns: &[String],
    protocol_columns: &[ColumnDef],
) -> Vec<String> {
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
                        let not_null_default = if field.required {
                            " NOT NULL DEFAULT ''"
                        } else {
                            ""
                        };
                        stmts.push(format!(
                            "ALTER TABLE {} ADD COLUMN {fk} INTEGER{not_null_default} REFERENCES {target_table}(id)",
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
                sql.push_str(" NOT NULL DEFAULT ''");
            }

            if let Some(ref default) = field.default {
                sql.push_str(&format!(" DEFAULT {}", json_to_sql_literal(default)));
            }

            stmts.push(sql);
        }
    }

    for col in protocol_columns {
        if !existing.contains(col.name.as_str()) {
            let mut sql = format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                ct.table,
                col.name,
                col.sql_type.as_str()
            );
            if let Some(ref default) = col.default {
                sql.push_str(&format!(" NOT NULL DEFAULT {}", default));
            }
            stmts.push(sql);
        }
    }

    stmts
}

/// 获取 content type schema 期望的所有列名（用于与 DB 对比）
#[must_use]
pub fn expected_columns(ct: &ContentTypeSchema, protocol_columns: &[ColumnDef]) -> Vec<String> {
    let mut cols = vec!["id".to_string(), "document_id".to_string()];

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

    for col in protocol_columns {
        cols.push(col.name.clone());
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
    use crate::aspects::SqlType;
    use crate::constants::*;

    fn default_protocol_columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_CREATED_AT.into(),
                sql_type: SqlType::Text,
                default: None,
            },
            ColumnDef {
                name: COL_UPDATED_AT.into(),
                sql_type: SqlType::Text,
                default: None,
            },
            ColumnDef {
                name: COL_CREATED_BY.into(),
                sql_type: SqlType::Text,
                default: None,
            },
            ColumnDef {
                name: COL_UPDATED_BY.into(),
                sql_type: SqlType::Text,
                default: None,
            },
        ]
    }

    fn soft_delete_protocol_columns() -> Vec<ColumnDef> {
        let mut cols = default_protocol_columns();
        cols.push(ColumnDef {
            name: COL_DELETED_AT.into(),
            sql_type: SqlType::Text,
            default: None,
        });
        cols.push(ColumnDef {
            name: COL_DELETED_BY.into(),
            sql_type: SqlType::Text,
            default: None,
        });
        cols
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

[fields.name]
type = "text"
required = true

[fields.slug]
type = "uid"
unique = true
"#,
        )
        .unwrap();

        let sql = generate_create_table(&ct, &default_protocol_columns());
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS tags"));
        assert!(sql.contains("document_id TEXT NOT NULL UNIQUE"));
        assert!(sql.contains("name TEXT NOT NULL"));
        assert!(sql.contains("created_at TEXT"));
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
implements = ["soft_deletable"]

[fields.name]
type = "text"
required = true
"#,
        )
        .unwrap();

        let sql = generate_create_table(&ct, &soft_delete_protocol_columns());
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

        let existing = vec![
            "id".into(),
            "document_id".into(),
            "title".into(),
            "slug".into(),
            "content".into(),
            "created_at".into(),
            "updated_at".into(),
            "created_by".into(),
            "updated_by".into(),
        ];

        let stmts = generate_alter_table(&ct, &existing, &default_protocol_columns());
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
            "document_id".into(),
            "name".into(),
            "slug".into(),
            "created_at".into(),
            "updated_at".into(),
            "created_by".into(),
            "updated_by".into(),
        ];

        let stmts = generate_alter_table(&ct, &existing, &default_protocol_columns());
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
implements = ["soft_deletable"]

[fields.title]
type = "text"
required = true
"#,
        )
        .unwrap();

        let existing = vec!["id".into(), "document_id".into(), "title".into()];

        let stmts = generate_alter_table(&ct, &existing, &soft_delete_protocol_columns());
        assert!(stmts.iter().any(|s| s.contains("created_at TEXT")));
        assert!(stmts.iter().any(|s| s.contains("updated_at TEXT")));
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

        let cols = expected_columns(&ct, &default_protocol_columns());
        assert!(cols.contains(&"id".to_string()));
        assert!(cols.contains(&"document_id".to_string()));
        assert!(cols.contains(&"title".to_string()));
        assert!(cols.contains(&"author_id".to_string()));
        assert!(cols.contains(&"created_by".to_string()));
        assert!(cols.contains(&"updated_by".to_string()));
        assert!(cols.contains(&"created_at".to_string()));
    }
}

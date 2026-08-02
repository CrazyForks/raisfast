//! Schema → SQL migration generator
//!
//! Automatically generates CREATE TABLE / ALTER TABLE SQL from `ContentTypeSchema` definitions.

use super::schema::{ContentTypeSchema, FieldType, RelationType};

use crate::aspects::ColumnDef;
use crate::constants::COL_ID;
use crate::db::DbDriver;

/// Generate CREATE TABLE SQL from a content type definition
///
/// `protocol_columns` is obtained from `ProtocolRegistry::columns_for()`,
/// containing all columns declared by implemented protocols (e.g. ownable, timestampable, soft_deletable).
#[must_use]
pub fn generate_create_table(ct: &ContentTypeSchema, protocol_columns: &[ColumnDef]) -> String {
    let table = ct.table.as_str();
    if !crate::db::driver::is_safe_identifier(table) {
        return format!("-- unsafe table name skipped: {table}");
    }
    let mut cols = Vec::new();

    cols.push(format!("    {COL_ID} {}", crate::db::Driver::pk_type()));

    for field in &ct.fields {
        if field.field_type == FieldType::Relation {
            match field.relation.as_ref().map(|r| &r.relation_type) {
                Some(RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay) => {
                    let fk = field
                        .relation
                        .as_ref()
                        .and_then(|r| r.foreign_key.clone())
                        .unwrap_or_else(|| format!("{}_id", field.name));
                    let target_table = field
                        .relation
                        .as_ref()
                        .map_or("users", |r| r.target.as_str());
                    if !crate::db::driver::is_safe_identifier(&fk)
                        || !crate::db::driver::is_safe_identifier(target_table)
                    {
                        continue;
                    }
                    let not_null = if field.required { " NOT NULL" } else { "" };
                    let fk_type = crate::db::Driver::fk_type();
                    cols.push(format!(
                        "    {fk} {fk_type}{not_null} REFERENCES {target_table}(id)"
                    ));
                }
                Some(RelationType::ManyToMany | RelationType::ManyWay) => {
                    // junction table generated separately below
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

/// Generate CREATE TABLE SQL for many-to-many junction tables
#[must_use]
pub fn generate_junction_tables(ct: &ContentTypeSchema) -> Vec<String> {
    let mut tables = Vec::new();

    let is_safe = crate::db::driver::is_safe_identifier;
    for field in &ct.fields {
        if let Some(ref rel) = field.relation
            && matches!(
                rel.relation_type,
                RelationType::ManyToMany | RelationType::ManyWay
            )
        {
            let through = rel
                .through
                .clone()
                .unwrap_or_else(|| format!("{}_{}", ct.table, rel.target));
            let source_col = format!("{}_id", ct.singular);
            let target_col = format!("{}_id", rel.target);

            if !is_safe(&through)
                || !is_safe(ct.table.as_str())
                || !is_safe(rel.target.as_str())
                || !is_safe(&source_col)
                || !is_safe(&target_col)
            {
                continue;
            }

            let fk_t = crate::db::Driver::fk_type();
            let sql = format!(
                "CREATE TABLE IF NOT EXISTS {through} (\n\
                 {source_col} {fk_t} NOT NULL REFERENCES {source_table}({COL_ID}),\n\
                 {target_col} {fk_t} NOT NULL REFERENCES {target_table}({COL_ID}),\n\
                 PRIMARY KEY ({source_col}, {target_col})\n\
                 )",
                fk_t = fk_t,
                through = through,
                source_col = source_col,
                source_table = ct.table,
                target_col = target_col,
                target_table = rel.target,
            );
            tables.push(sql);
        }
    }

    tables
}

/// Generate CREATE INDEX SQL for indexes
#[must_use]
pub fn generate_indexes(ct: &ContentTypeSchema) -> Vec<String> {
    let mut indexes = Vec::new();

    for field in &ct.fields {
        if field.unique && field.field_type != FieldType::Uid {
            let idx_name = format!("idx_{}_{}_unique", ct.table, field.name);
            indexes.push(create_index_sql(&idx_name, &ct.table, &field.name, true));
        }
    }

    for idx in &ct.indexes {
        let cols = idx.fields.join(",");
        if idx.unique {
            let idx_name = format!("idx_{}_{}_unique", ct.table, idx.fields.join("_"));
            indexes.push(create_index_sql(&idx_name, &ct.table, &cols, true));
        } else {
            let idx_name = format!("idx_{}_{}", ct.table, idx.fields.join("_"));
            indexes.push(create_index_sql(&idx_name, &ct.table, &cols, false));
        }
    }

    indexes
}

fn create_index_sql(name: &str, table: &str, columns: &str, unique: bool) -> String {
    let unique_kw = if unique { "UNIQUE " } else { "" };
    #[cfg(feature = "db-mysql")]
    {
        format!("CREATE {unique_kw}INDEX {name} ON {table}({columns})")
    }
    #[cfg(not(feature = "db-mysql"))]
    {
        format!("CREATE {unique_kw}INDEX IF NOT EXISTS {name} ON {table}({columns})")
    }
}

/// Generate ALTER TABLE ADD COLUMN SQL from a content type definition and existing columns
///
/// Compares expected schema columns with existing database columns, generating DDL only for missing ones.
/// Does not drop or alter column types — only incremental additions (consistent with Strapi's `forceMigration` strategy).
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

    // id column should never be missing (table exists means primary key present), skip

    for field in &ct.fields {
        if field.field_type == FieldType::Relation {
            match field.relation.as_ref().map(|r| &r.relation_type) {
                Some(RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay) => {
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
                            "ALTER TABLE {} ADD COLUMN {fk} INTEGER{not_null_default} REFERENCES {target_table}({COL_ID})",
                            ct.table
                        ));
                    }
                }
                Some(RelationType::ManyToMany | RelationType::ManyWay) => {}
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

/// Detect type mismatches between schema and existing DB columns.
///
/// Returns a list of `(column_name, expected_type, actual_type)` tuples for columns
/// whose SQL type differs from what the schema expects.
#[must_use]
pub fn detect_type_mismatches(
    ct: &ContentTypeSchema,
    db_columns: &[(String, String)],
    protocol_columns: &[ColumnDef],
) -> Vec<(String, String, String)> {
    let db_map: std::collections::HashMap<&str, &str> = db_columns
        .iter()
        .map(|(name, typ)| (name.as_str(), typ.as_str()))
        .collect();

    let mut mismatches = Vec::new();

    for field in &ct.fields {
        let col_name = if field.field_type == FieldType::Relation {
            match field.relation.as_ref().map(|r| &r.relation_type) {
                Some(RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay) => {
                    field
                        .relation
                        .as_ref()
                        .and_then(|r| r.foreign_key.clone())
                        .unwrap_or_else(|| format!("{}_id", field.name))
                }
                _ => continue,
            }
        } else {
            field.name.clone()
        };

        let expected = field_type_to_sql(&field.field_type);
        if let Some(actual) = db_map.get(col_name.as_str())
            && !type_compatible(expected, actual)
        {
            mismatches.push((col_name, expected.to_string(), (*actual).to_string()));
        }
    }

    for col in protocol_columns {
        if let Some(actual) = db_map.get(col.name.as_str())
            && !type_compatible(col.sql_type.as_str(), actual)
        {
            mismatches.push((
                col.name.clone(),
                col.sql_type.as_str().to_string(),
                (*actual).to_string(),
            ));
        }
    }

    mismatches
}

/// Generate a rebuild-table migration SQL for type mismatches.
///
/// Produces database-specific SQL based on the active `db-*` feature flag:
/// - **SQLite**: PRAGMA foreign_keys=OFF, BEGIN, CREATE new, INSERT, DROP, RENAME, PRAGMA foreign_key_check, COMMIT
/// - **PostgreSQL**: BEGIN, CREATE new, INSERT, DROP, RENAME, COMMIT
/// - **MySQL**: SET FOREIGN_KEY_CHECKS=0, CREATE new, INSERT, DROP, RENAME, SET FOREIGN_KEY_CHECKS=1
#[must_use]
pub fn generate_rebuild_migration(
    ct: &ContentTypeSchema,
    mismatches: &[(String, String, String)],
    protocol_columns: &[ColumnDef],
) -> String {
    let table = ct.table.as_str();
    let temp = format!("{table}__new");

    let create = generate_create_table_for_rebuild(ct, protocol_columns, &temp);
    let indexes = generate_indexes(ct);

    let mut sql = String::new();
    sql.push_str("-- Auto-generated migration to fix column type mismatches\n");
    sql.push_str(&format!(
        "-- Table: {table}, Content type: {}\n",
        ct.singular
    ));
    sql.push_str("-- Mismatches:\n");
    for (col, expected, actual) in mismatches {
        sql.push_str(&format!("--   {col}: {actual} -> {expected}\n"));
    }
    sql.push_str("--\n");
    sql.push_str("-- REVIEW THIS FILE BEFORE EXECUTING!\n");
    sql.push_str(
        "-- Data in mismatched columns may be lost or truncated during type conversion.\n",
    );
    sql.push_str("-- Back up your database before running.\n");
    sql.push_str("--\n\n");

    sql.push_str(&crate::db::Driver::rebuild_wrapper_sql(
        table,
        &temp,
        &format!("{create}\n"),
        &indexes,
    ));

    sql
}

fn generate_create_table_for_rebuild(
    ct: &ContentTypeSchema,
    protocol_columns: &[ColumnDef],
    table_name: &str,
) -> String {
    let mut cols = Vec::new();

    cols.push(format!("    {COL_ID} {}", crate::db::Driver::pk_type()));

    for field in &ct.fields {
        if field.field_type == FieldType::Relation {
            match field.relation.as_ref().map(|r| &r.relation_type) {
                Some(RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay) => {
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
                    let fk_type = crate::db::Driver::fk_type();
                    cols.push(format!(
                        "    {fk} {fk_type}{not_null} REFERENCES {target_table}(id)"
                    ));
                }
                Some(RelationType::OneToMany) => {}
                _ => {}
            }
            continue;
        }

        let col_type = field_type_to_sql(&field.field_type);
        let mut col_def = format!("    {} {}", field.name, col_type);
        if field.required {
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

    let mut sql = format!("CREATE TABLE {table_name} (\n");
    sql.push_str(&cols.join(",\n"));
    sql.push_str("\n);\n\n");

    sql
}

fn type_compatible(expected: &str, actual: &str) -> bool {
    let normalize = |s: &str| -> String {
        let s = s.to_uppercase();
        let s = s.split('(').next().unwrap_or(&s).trim();
        match s {
            "INT" | "INTEGER" | "BIGINT" => "INTEGER",
            "TEXT" | "VARCHAR" | "CHAR" => "TEXT",
            "REAL" | "FLOAT" | "DOUBLE" | "NUMERIC" => "REAL",
            "BOOLEAN" | "BOOL" => "BOOLEAN",
            "DECIMAL" => "DECIMAL",
            other => other,
        }
        .to_string()
    };
    normalize(expected) == normalize(actual)
}

/// Get all column names expected by the content type schema (for comparison with DB)
#[must_use]
pub fn expected_columns(ct: &ContentTypeSchema, protocol_columns: &[ColumnDef]) -> Vec<String> {
    let mut cols = vec![COL_ID.to_string()];

    for field in &ct.fields {
        if field.field_type == FieldType::Relation {
            if let Some(RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay) =
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
    use crate::db::sql_type::SqlType;
    match ft {
        FieldType::Text => SqlType::Varchar.as_str(),
        FieldType::RichText => SqlType::Text.as_str(),
        FieldType::Json => SqlType::Json.as_str(),
        FieldType::Integer => SqlType::Integer.as_str(),
        FieldType::BigInt => SqlType::BigInt.as_str(),
        FieldType::Float => SqlType::Real.as_str(),
        FieldType::Decimal => SqlType::Decimal.as_str(),
        FieldType::Boolean => SqlType::Boolean.as_str(),
        FieldType::Date => SqlType::Date.as_str(),
        FieldType::DateTime => SqlType::Timestamp.as_str(),
        FieldType::Time => SqlType::Time.as_str(),
        FieldType::Email
        | FieldType::Password
        | FieldType::Enum
        | FieldType::Uid
        | FieldType::Media => SqlType::Varchar.as_str(),
        FieldType::Relation => SqlType::BigInt.as_str(),
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
    use crate::constants::*;
    use crate::db::sql_type::SqlType;

    fn default_protocol_columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_CREATED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
            ColumnDef {
                name: COL_UPDATED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
            ColumnDef {
                name: COL_CREATED_BY.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
            ColumnDef {
                name: COL_UPDATED_BY.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
        ]
    }

    fn soft_delete_protocol_columns() -> Vec<ColumnDef> {
        let mut cols = default_protocol_columns();
        cols.push(ColumnDef {
            name: COL_DELETED_AT.into(),
            sql_type: SqlType::Timestamp,
            default: None,
        });
        cols.push(ColumnDef {
            name: COL_DELETED_BY.into(),
            sql_type: SqlType::BigInt,
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
        assert!(sql.contains("name TEXT NOT NULL"));
        assert!(sql.contains("created_at TEXT"));
        assert!(!sql.contains("status"));
        assert!(!sql.contains("document_id"));
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

        let existing = vec!["id".into(), "title".into()];

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
        assert!(cols.contains(&"title".to_string()));
        assert!(cols.contains(&"author_id".to_string()));
        assert!(cols.contains(&"created_by".to_string()));
        assert!(cols.contains(&"updated_by".to_string()));
        assert!(cols.contains(&"created_at".to_string()));
    }

    #[test]
    fn field_type_to_sql_all_types() {
        assert_eq!(field_type_to_sql(&FieldType::Text), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::RichText), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Integer), "INTEGER");
        assert_eq!(field_type_to_sql(&FieldType::BigInt), "INTEGER");
        assert_eq!(field_type_to_sql(&FieldType::Decimal), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Float), "REAL");
        assert_eq!(field_type_to_sql(&FieldType::Boolean), "BOOLEAN");
        assert_eq!(field_type_to_sql(&FieldType::Date), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::DateTime), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Time), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Email), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Password), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Enum), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Uid), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Json), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Media), "TEXT");
        assert_eq!(field_type_to_sql(&FieldType::Relation), "INTEGER");
    }

    #[test]
    fn json_to_sql_literal_variants() {
        assert_eq!(json_to_sql_literal(&serde_json::json!("hello")), "'hello'");
        assert_eq!(json_to_sql_literal(&serde_json::json!("it's")), "'it''s'");
        assert_eq!(json_to_sql_literal(&serde_json::json!(42)), "42");
        assert_eq!(json_to_sql_literal(&serde_json::json!(true)), "1");
        assert_eq!(json_to_sql_literal(&serde_json::json!(false)), "0");
        assert_eq!(json_to_sql_literal(&serde_json::json!(null)), "NULL");
        assert_eq!(json_to_sql_literal(&serde_json::json!([1, 2])), "'[1,2]'");
    }

    #[test]
    fn generate_create_table_with_one_to_one_relation() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Profile"
singular = "profile"
plural = "profiles"
table = "profiles"

[fields.user]
type = "relation"
relation_type = "one_to_one"
target = "users"
"#,
        )
        .unwrap();
        let sql = generate_create_table(&ct, &[]);
        assert!(sql.contains("user_id INTEGER REFERENCES users(id)"));
    }

    #[test]
    fn generate_create_table_with_required_relation() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
required = true
"#,
        )
        .unwrap();
        let sql = generate_create_table(&ct, &[]);
        assert!(sql.contains("NOT NULL"));
        assert!(sql.contains("REFERENCES users(id)"));
    }

    #[test]
    fn generate_create_table_with_custom_fk() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
foreign_key = "owner_id"
"#,
        )
        .unwrap();
        let sql = generate_create_table(&ct, &[]);
        assert!(sql.contains("owner_id INTEGER REFERENCES users(id)"));
    }

    #[test]
    fn generate_create_table_m2m_no_fk_column() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
"#,
        )
        .unwrap();
        let sql = generate_create_table(&ct, &[]);
        assert!(!sql.contains("tags_id"));
        assert!(!sql.contains("tags"));
    }

    #[test]
    fn generate_junction_tables_basic() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
through = "posts_tags"
"#,
        )
        .unwrap();
        let tables = generate_junction_tables(&ct);
        assert_eq!(tables.len(), 1);
        assert!(tables[0].contains("CREATE TABLE IF NOT EXISTS posts_tags"));
        assert!(tables[0].contains("post_id"));
        assert!(tables[0].contains("tags_id"));
        assert!(tables[0].contains("PRIMARY KEY"));
    }

    #[test]
    fn generate_junction_tables_auto_name() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
"#,
        )
        .unwrap();
        let tables = generate_junction_tables(&ct);
        assert_eq!(tables.len(), 1);
        assert!(tables[0].contains("posts_tags"));
    }

    #[test]
    fn generate_junction_tables_empty_when_no_m2m() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
"#,
        )
        .unwrap();
        let tables = generate_junction_tables(&ct);
        assert!(tables.is_empty());
    }

    #[test]
    fn generate_indexes_unique_field() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Tag"
singular = "tag"
plural = "tags"
table = "tags"

[fields.slug]
type = "uid"
unique = true

[fields.name]
type = "text"
unique = true
"#,
        )
        .unwrap();
        let indexes = generate_indexes(&ct);
        assert!(indexes.iter().any(|s| s.contains("idx_tags_name_unique")));
        assert!(!indexes.iter().any(|s| s.contains("slug")));
    }

    #[test]
    fn generate_indexes_composite() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"

[[indexes]]
fields = ["title", "slug"]
unique = true

[[indexes]]
fields = ["title"]
"#,
        )
        .unwrap();
        let indexes = generate_indexes(&ct);
        assert_eq!(indexes.len(), 2);
        assert!(indexes[0].contains("UNIQUE"));
        assert!(!indexes[1].contains("UNIQUE"));
    }

    #[test]
    fn expected_columns_with_m2m_no_fk() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
"#,
        )
        .unwrap();
        let cols = expected_columns(&ct, &[]);
        assert!(cols.contains(&"title".to_string()));
        assert!(!cols.contains(&"tags_id".to_string()));
    }

    #[test]
    fn generate_create_table_with_boolean_default() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.active]
type = "boolean"
required = true
default = true
"#,
        )
        .unwrap();
        let sql = generate_create_table(&ct, &[]);
        assert!(sql.contains("active BOOLEAN DEFAULT 1"));
        let active_line = sql.lines().find(|l| l.contains("active BOOLEAN")).unwrap();
        assert!(!active_line.contains("NOT NULL"));
    }

    #[test]
    fn generate_create_table_with_default_string() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.status]
type = "text"
default = "draft"
"#,
        )
        .unwrap();
        let sql = generate_create_table(&ct, &[]);
        assert!(sql.contains("DEFAULT 'draft'"));
    }

    #[test]
    fn generate_create_table_protocol_col_not_duplicated() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.created_at]
type = "text"
"#,
        )
        .unwrap();
        let proto = vec![ColumnDef {
            name: "created_at".into(),
            sql_type: SqlType::Timestamp,
            default: None,
        }];
        let sql = generate_create_table(&ct, &proto);
        let count = sql.matches("created_at").count();
        assert_eq!(count, 1, "created_at should appear exactly once");
    }
}

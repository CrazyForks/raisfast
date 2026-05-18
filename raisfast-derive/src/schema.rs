//! SQL schema parser and dialect detection for compile-time validation.
//!
//! This module:
//! 1. Detects the database dialect from `DATABASE_URL` at proc-macro expansion time.
//! 2. Reads the appropriate migration files and builds an in-memory schema representation.
//! 3. Provides placeholder generation, pagination syntax, and schema file paths per dialect.
//!
//! # Dialect detection
//!
//! The dialect is inferred from the `DATABASE_URL` environment variable prefix:
//! - `sqlite:` → `Sqlite`
//! - `postgres:` or `postgresql:` → `Postgres`
//! - `mysql:` → `Mysql`
//!
//! # Files parsed
//!
//! Depending on the dialect, one of:
//! - `migrations/sqlite/schema.sqlite.sql` + `tenantable.sqlite.sql`
//! - `migrations/postgres/schema.postgres.sql` + `tenantable.postgres.sql`
//! - `migrations/mysql/schema.mysql.sql` + `tenantable.mysql.sql`

use std::collections::HashMap;
use std::path::PathBuf;

/// Supported database dialects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
}

impl Dialect {
    /// Detect dialect from `DATABASE_URL` environment variable.
    ///
    /// Falls back to `Sqlite` if the variable is unset or unrecognized.
    pub fn from_env() -> Self {
        let url = std::env::var("DATABASE_URL").unwrap_or_default();
        if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Dialect::Postgres
        } else if url.starts_with("mysql:") {
            Dialect::Mysql
        } else {
            Dialect::Sqlite
        }
    }

    /// Numbered placeholder for position `idx` (1-based).
    ///
    /// - SQLite: `?1`, `?2`, ...
    /// - PostgreSQL: `$1`, `$2`, ...
    /// - MySQL: `?1`, `?2`, ... (MySQL can also use `?` but numbered is fine)
    pub fn ph(&self, idx: usize) -> String {
        match self {
            Dialect::Sqlite | Dialect::Mysql => format!("?{}", idx),
            Dialect::Postgres => format!("${}", idx),
        }
    }

    /// Unnumbered placeholder token for runtime `sqlx::query()` calls.
    ///
    /// - SQLite / MySQL: `?`
    /// - PostgreSQL: Cannot use unnumbered; caller must use `ph(idx)` instead.
    #[expect(dead_code)]
    pub fn ph_unnumbered(&self) -> &'static str {
        match self {
            Dialect::Sqlite | Dialect::Mysql => "?",
            Dialect::Postgres => "$?", // intentionally wrong — postgres path uses ph()
        }
    }

    /// Pagination clause appended to a query.
    ///
    /// - SQLite / PostgreSQL: ` LIMIT ? OFFSET ?`
    /// - MySQL: ` LIMIT ? OFFSET ?`
    #[expect(dead_code)]
    pub fn limit_offset_clause(&self) -> &'static str {
        " LIMIT ? OFFSET ?"
    }

    /// Schema migration directory name.
    pub fn migration_dir(&self) -> &'static str {
        match self {
            Dialect::Sqlite => "sqlite",
            Dialect::Postgres => "postgres",
            Dialect::Mysql => "mysql",
        }
    }

    /// Schema file extension (including leading dot).
    pub fn schema_ext(&self) -> &'static str {
        match self {
            Dialect::Sqlite => ".sqlite.sql",
            Dialect::Postgres => ".postgres.sql",
            Dialect::Mysql => ".mysql.sql",
        }
    }
}

/// The full database schema, keyed by lowercase table name.
pub struct Schema {
    pub tables: HashMap<String, TableSchema>,
    pub dialect: Dialect,
}

/// A single table's schema — just its ordered list of columns.
pub struct TableSchema {
    pub columns: Vec<ColumnSchema>,
}

/// Metadata for a single column.
pub struct ColumnSchema {
    pub name: String,
    #[expect(dead_code)]
    pub ty: SqlType,
    #[expect(dead_code)]
    pub nullable: bool,
    #[expect(dead_code)]
    pub has_default: bool,
}

/// Simplified SQL type classification.
pub enum SqlType {
    Integer,
    Real,
    Text,
    Blob,
}

impl Schema {
    /// Load and parse migration SQL files for the detected dialect.
    pub fn load() -> Self {
        let dialect = Dialect::from_env();
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
        let base = PathBuf::from(manifest_dir);
        let dir = format!("migrations/{}", dialect.migration_dir());
        let ext = dialect.schema_ext();

        let schema_file = format!("{}{}{}", dir, "/schema", ext);
        let schema_sql = std::fs::read_to_string(base.join(&schema_file)).unwrap_or_default();

        let tables = parse_schema(&schema_sql);

        Schema { tables, dialect }
    }

    /// Generate a SELECT column list with chrono type annotations for timestamp columns.
    ///
    /// Returns something like: `id, title, created_at as "created_at: chrono::DateTime<chrono::Utc>"`.
    /// This format is consumed by `sqlx::query!()` to get proper type inference.
    ///
    /// Currently unused (dead_code) — kept for potential future use.
    #[expect(dead_code)]
    pub fn select_columns(&self, table: &str) -> Option<String> {
        let ts = self.tables.get(table)?;
        let mut parts: Vec<String> = Vec::new();
        for col in &ts.columns {
            if is_timestamp_col(&col.name) {
                parts.push(format!(
                    r#"{} as "{}: chrono::DateTime<chrono::Utc>""#,
                    col.name, col.name
                ));
            } else {
                parts.push(col.name.clone());
            }
        }
        Some(parts.join(", "))
    }

    /// Return just the column names for a table, joined by `", "`.
    ///
    /// This is the primary method used by `crud.rs` to generate explicit `SELECT` column
    /// lists, replacing `SELECT *` (which `sqlx::query!()` does not support).
    pub fn column_names(&self, table: &str) -> Vec<String> {
        self.tables
            .get(table)
            .map(|ts| ts.columns.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    }
}

/// Check if a column name is a known timestamp column that needs chrono type annotation.
#[allow(dead_code)]
fn is_timestamp_col(name: &str) -> bool {
    name == "created_at" || name == "updated_at" || name == "expires_at"
}

/// Parse `CREATE TABLE` statements from SQL text into a HashMap.
///
/// Two-pass approach:
/// 1. First pass: discover all table names.
/// 2. Second pass: parse column definitions inside each CREATE TABLE block.
///
/// This handles multi-line CREATE TABLE statements with columns listed one per line.
fn parse_schema(sql: &str) -> HashMap<String, TableSchema> {
    let mut tables = HashMap::new();

    // Pass 1: discover table names
    for line in sql.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("CREATE TABLE")
            && let Some(name) = extract_table_name(rest)
        {
            tables.insert(
                name,
                TableSchema {
                    columns: Vec::new(),
                },
            );
        }
    }

    // Pass 2: parse column definitions
    let mut current_table: Option<String> = None;
    let mut in_create = false;

    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Detect start of CREATE TABLE block
        if let Some(rest) = trimmed.strip_prefix("CREATE TABLE") {
            if let Some(name) = extract_table_name(rest) {
                current_table = Some(name);
                in_create = true;
                if let Some(t) = tables.get_mut(current_table.as_ref().unwrap()) {
                    t.columns.clear();
                }
            }
            continue;
        }

        // Detect end of CREATE TABLE block (closing parenthesis)
        if in_create && trimmed.starts_with(')') {
            in_create = false;
            current_table = None;
            continue;
        }

        // Parse a column line inside a CREATE TABLE block
        if let (Some(tn), true) = (&current_table, in_create)
            && let Some(col) = parse_column_line(trimmed)
            && let Some(t) = tables.get_mut(tn)
        {
            t.columns.push(col);
        }
    }

    tables
}

/// Extract the table name from the text after `CREATE TABLE`.
///
/// Handles `IF NOT EXISTS` prefix. Returns the name lowercased.
fn extract_table_name(rest: &str) -> Option<String> {
    let rest = rest.trim();
    let rest = rest.strip_prefix("IF NOT EXISTS").unwrap_or(rest).trim();
    let name = rest
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()?;
    if name.is_empty() {
        return None;
    }
    Some(name.to_lowercase())
}

/// Parse a single column definition line from a CREATE TABLE block.
///
/// Skips constraint lines (PRIMARY KEY, UNIQUE, CHECK, FOREIGN KEY).
/// Extracts column name, SQL type, nullability, and default presence.
fn parse_column_line(line: &str) -> Option<ColumnSchema> {
    let line = line.trim().trim_end_matches(',');
    if line.is_empty()
        || line.starts_with("PRIMARY KEY")
        || line.starts_with("UNIQUE(")
        || line.starts_with("CHECK(")
        || line.starts_with("FOREIGN")
    {
        return None;
    }

    let mut parts = line.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    let rest = parts.next()?.trim().to_uppercase();

    if name.is_empty() {
        return None;
    }

    let ty = if rest.starts_with("INTEGER") || rest.starts_with("INT") {
        SqlType::Integer
    } else if rest.starts_with("REAL") || rest.starts_with("FLOAT") || rest.starts_with("DOUBLE") {
        SqlType::Real
    } else if rest.starts_with("BLOB") {
        SqlType::Blob
    } else {
        SqlType::Text
    };

    let nullable = !rest.contains("NOT NULL");
    let has_default = rest.contains("DEFAULT");

    Some(ColumnSchema {
        name: name.to_lowercase(),
        ty,
        nullable,
        has_default,
    })
}

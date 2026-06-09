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
//! # Adding a new database
//!
//! 1. Add a variant to `Dialect` enum
//! 2. Add a row to the `DIALECT_TABLE` const
//! 3. Add the URL prefix detection in `from_env()`
//! 4. Create `migrations/<name>/schema.<name>.sql`

use std::collections::HashMap;
use std::path::PathBuf;

/// Per-dialect configuration, indexed by `Dialect` discriminant.
struct DialectCfg {
    migration_dir: &'static str,
    schema_ext: &'static str,
    ph_prefix: &'static str,
}

/// Supported database dialects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
}

impl Dialect {
    const TABLE: &'static [DialectCfg] = &[
        // Sqlite
        DialectCfg {
            migration_dir: "sqlite",
            schema_ext: ".sqlite.sql",
            ph_prefix: "?",
        },
        // Postgres
        DialectCfg {
            migration_dir: "postgres",
            schema_ext: ".postgres.sql",
            ph_prefix: "$",
        },
        // Mysql
        DialectCfg {
            migration_dir: "mysql",
            schema_ext: ".mysql.sql",
            ph_prefix: "?",
        },
    ];

    fn cfg(self) -> &'static DialectCfg {
        &Self::TABLE[self as usize]
    }

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
    pub fn ph(&self, idx: usize) -> String {
        match self {
            Dialect::Postgres => format!("${idx}"),
            Dialect::Sqlite => format!("?{idx}"),
            Dialect::Mysql => "?".to_string(),
        }
    }

    /// Quote a SQL identifier (table name, column name) for the current dialect.
    ///
    /// MySQL uses backticks, PostgreSQL uses double quotes, SQLite accepts both.
    pub fn qi(&self, name: &str) -> String {
        match self {
            Dialect::Mysql => format!("`{name}`"),
            Dialect::Postgres => format!("\"{name}\""),
            Dialect::Sqlite => name.to_string(),
        }
    }

    pub fn qi_col(&self, col: &str) -> String {
        if col.contains('.') {
            col.split('.')
                .map(|part| self.qi(part))
                .collect::<Vec<_>>()
                .join(".")
        } else {
            self.qi(col)
        }
    }

    /// Quote and comma-join a list of identifiers.
    pub fn qi_list(&self, names: &[String]) -> String {
        names
            .iter()
            .map(|n| self.qi(n))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Unnumbered placeholder token for runtime `sqlx::query()` calls.
    #[expect(dead_code)]
    pub fn ph_unnumbered(&self) -> &'static str {
        let c = self.cfg();
        if c.ph_prefix == "$" {
            "$?" // intentionally wrong — postgres path uses ph()
        } else {
            "?"
        }
    }

    /// Pagination clause appended to a query.
    #[expect(dead_code)]
    pub fn limit_offset_clause(&self) -> &'static str {
        " LIMIT ? OFFSET ?"
    }

    /// Schema migration directory name.
    pub fn migration_dir(&self) -> &'static str {
        self.cfg().migration_dir
    }

    /// Schema file extension (including leading dot).
    pub fn schema_ext(&self) -> &'static str {
        self.cfg().schema_ext
    }

    /// UPSERT conflict clause.
    pub fn upsert_clause(&self, conflict_cols: &str, assignments: &str) -> String {
        match self {
            Dialect::Mysql => format!("ON DUPLICATE KEY UPDATE {assignments}"),
            _ => format!("ON CONFLICT({conflict_cols}) DO UPDATE SET {assignments}"),
        }
    }

    /// Reference to the new/excluded value in an UPSERT.
    pub fn excluded_col(&self, col: &str) -> String {
        match self {
            Dialect::Mysql => format!("VALUES({})", self.qi(col)),
            _ => format!("excluded.{}", self.qi(col)),
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

    /// Return just the column names for a table, quoted for the current dialect.
    pub fn column_names(&self, table: &str) -> Vec<String> {
        self.tables
            .get(table)
            .map(|ts| {
                ts.columns
                    .iter()
                    .map(|c| self.dialect.qi(&c.name))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[allow(dead_code)]
fn is_timestamp_col(name: &str) -> bool {
    name == "created_at" || name == "updated_at" || name == "expires_at"
}

fn parse_schema(sql: &str) -> HashMap<String, TableSchema> {
    let mut tables = HashMap::new();

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

    let mut current_table: Option<String> = None;
    let mut in_create = false;

    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

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

        if in_create && trimmed.starts_with(')') {
            in_create = false;
            current_table = None;
            continue;
        }

        if let (Some(tn), true) = (&current_table, in_create)
            && let Some(col) = parse_column_line(trimmed)
            && let Some(t) = tables.get_mut(tn)
        {
            t.columns.push(col);
        }
    }

    tables
}

fn extract_table_name(rest: &str) -> Option<String> {
    let rest = rest.trim();
    let rest = rest.strip_prefix("IF NOT EXISTS").unwrap_or(rest).trim();
    let name = rest
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '`')
        .next()?;
    let name = name.trim_matches('`');
    if name.is_empty() {
        return None;
    }
    Some(name.to_lowercase())
}

fn parse_column_line(line: &str) -> Option<ColumnSchema> {
    let line = line.trim().trim_end_matches(',');
    if line.is_empty()
        || line.starts_with("PRIMARY KEY")
        || line.starts_with("UNIQUE(")
        || line.starts_with("CHECK(")
        || line.starts_with("FOREIGN")
        || line.starts_with("INDEX")
        || line.starts_with("UNIQUE INDEX")
        || line.starts_with("CONSTRAINT")
    {
        return None;
    }

    let mut parts = line.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim().trim_matches('`');
    let rest = parts.next()?.trim().to_uppercase();

    if name.is_empty() {
        return None;
    }

    if !rest.starts_with("INTEGER")
        && !rest.starts_with("INT")
        && !rest.starts_with("BIGINT")
        && !rest.starts_with("TINYINT")
        && !rest.starts_with("SMALLINT")
        && !rest.starts_with("REAL")
        && !rest.starts_with("FLOAT")
        && !rest.starts_with("DOUBLE")
        && !rest.starts_with("DECIMAL")
        && !rest.starts_with("NUMERIC")
        && !rest.starts_with("TEXT")
        && !rest.starts_with("LONGTEXT")
        && !rest.starts_with("MEDIUMTEXT")
        && !rest.starts_with("VARCHAR")
        && !rest.starts_with("CHAR")
        && !rest.starts_with("BLOB")
        && !rest.starts_with("LONGBLOB")
        && !rest.starts_with("MEDIUMBLOB")
        && !rest.starts_with("TINYBLOB")
        && !rest.starts_with("JSON")
        && !rest.starts_with("BOOLEAN")
        && !rest.starts_with("BOOL")
        && !rest.starts_with("DATETIME")
        && !rest.starts_with("DATE")
        && !rest.starts_with("TIMESTAMP")
        && !rest.starts_with("TIME")
    {
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

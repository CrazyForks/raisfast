//! Database schema constants.
//!
//! Embedded at compile time from `migrations/{db}/schema.{db}.sql`, used for testing and initialization.
//! The appropriate schema file is selected based on the compile-time feature flag.

use std::sync::LazyLock;

#[cfg(feature = "db-sqlite")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/sqlite/schema.sqlite.sql");

#[cfg(feature = "db-postgres")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/postgres/schema.postgres.sql");

#[cfg(feature = "db-mysql")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/mysql/schema.mysql.sql");

fn parse_create_table_names(sql: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pos = 0;
    while let Some(idx) = sql[pos..].find("CREATE TABLE") {
        let start = pos + idx + "CREATE TABLE".len();
        let rest = sql[start..].trim_start();
        let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
        let end = rest.find([' ', '(', '\n']).unwrap_or(rest.len());
        if end > 0 {
            names.push(rest[..end].to_string());
        }
        pos = start + 1;
    }
    names
}

/// All table names in the schema + `_migrations`, generated once at first access.
pub static SCHEMA_TABLES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut tables = parse_create_table_names(SCHEMA_SQL);
    if !tables.iter().any(|t| t == "_migrations") {
        tables.push("_migrations".to_string());
    }
    tables
});

static PROTECTED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Compute and cache the final protected table list at startup.
///
/// Takes all live tables from the database and subtracts content-type tables.
/// Called once from `build_app_state()` after content types are loaded.
/// Subsequent calls are silently ignored (idempotent).
pub fn set_protected_tables(live_tables: Vec<String>, ct_tables: &[String]) {
    let ct_set: std::collections::HashSet<&str> = ct_tables.iter().map(|s| s.as_str()).collect();
    let protected: Vec<String> = live_tables
        .into_iter()
        .filter(|t| !ct_set.contains(t.as_str()))
        .collect();
    let _ = PROTECTED.set(protected);
}

/// Return the protected table list.
///
/// At runtime returns the list computed by `set_protected_tables()` (live DB tables
/// minus content-type tables). Falls back to the compile-time `SCHEMA_TABLES` when
/// no database is available (e.g. unit tests).
pub fn get_protected_tables() -> Vec<String> {
    PROTECTED
        .get()
        .cloned()
        .unwrap_or_else(|| SCHEMA_TABLES.clone())
}

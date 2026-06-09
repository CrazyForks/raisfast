//! Unified database driver abstraction.
//!
//! A sealed trait [`DbDriver`] with exactly one compile-time implementation.
//! Adding a new database backend requires only:
//! 1. Add a zero-sized marker struct and `impl sealed::Sealed for X`
//! 2. `impl DbDriver for X` (~100 lines)
//! 3. Add the cfg-selected type alias `type Driver = X`
//! 4. Add pool type aliases in `pool.rs`
//!
//! Business code calls `Driver::ph(1)`, `Driver::has_column(...)`, etc.
//! The compiler resolves these to the single active impl — zero runtime cost.

use super::pool::Pool;

mod sealed {
    pub trait Sealed {}
}

pub struct Sqlite;
pub struct Postgres;
pub struct MySql;

impl sealed::Sealed for Sqlite {}
impl sealed::Sealed for Postgres {}
impl sealed::Sealed for MySql {}

#[cfg(feature = "db-sqlite")]
pub type Driver = Sqlite;
#[cfg(feature = "db-postgres")]
pub type Driver = Postgres;
#[cfg(feature = "db-mysql")]
pub type Driver = MySql;

/// Check if an identifier contains only safe characters (letters, digits, underscores).
#[must_use]
pub fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Trim and validate an identifier. Returns `Some(trimmed)` if valid, `None` otherwise.
#[must_use]
pub fn sanitize_identifier(name: &str) -> Option<&str> {
    let trimmed = name.trim();
    if is_safe_identifier(trimmed) {
        Some(trimmed)
    } else {
        None
    }
}

/// All database-specific behavior in one sealed trait.
///
/// Call as `Driver::method(...)` — resolves to the compile-time selected backend.
pub trait DbDriver: sealed::Sealed {
    // ── DDL types ────────────────────────────────────────────────

    /// PRIMARY KEY column type (`INTEGER` / `BIGINT`).
    fn pk_type() -> &'static str;

    /// Foreign key column type (derived from `pk_type`).
    fn fk_type() -> &'static str;

    // ── SQL dialect (sync) ──────────────────────────────────────

    /// Positional placeholder (`?` or `$N`).
    fn ph(idx: usize) -> String;

    /// Current timestamp function.
    fn now_fn() -> &'static str;

    /// Expression for `N days ago`.
    fn ago_expr(days: i64) -> String;

    /// Truncate a datetime column to day granularity.
    fn date_trunc_day(col: &str) -> String;

    /// Current date expression for comparing with `date_trunc_day`.
    fn current_date() -> String;

    /// UPSERT conflict clause.
    fn upsert_clause(conflict_cols: &str, assignments: &str) -> String;

    /// Reference to the new/excluded value in an UPSERT.
    fn excluded_col(col: &str) -> String;

    /// Cast an expression to a signed integer type.
    ///
    /// MySQL's `COUNT(*)` and `SUM()` return `DECIMAL`, which `i64` cannot decode.
    /// This wraps the expression in `CAST(... AS SIGNED)` on MySQL; other backends return it as-is.
    fn cast_int(expr: &str) -> String;

    /// Cast a datetime expression to CHAR for decoding as Rust `String`.
    ///
    /// MySQL returns `DATETIME`/`DATE` which sqlx cannot decode into `String`.
    /// SQLite and PostgreSQL store timestamps as TEXT already.
    fn cast_ts(expr: &str) -> String;

    /// `RETURNING *` clause (empty string on MySQL).
    fn returning_clause() -> &'static str;

    /// `RETURNING {col}` clause (empty string on MySQL).
    fn returning_col(col: &str) -> String;

    /// INSERT IGNORE (or equivalent) full statement.
    fn insert_ignore_sql(table: &str, columns: &str, placeholders: &str) -> String;

    // ── Metadata SQL builders (sync) ────────────────────────────

    /// SQL + column-name-index for listing columns with types.
    fn columns_sql(table: &str) -> (String, usize);

    /// SQL + column-name-index for listing column names only.
    fn column_names_sql(table: &str) -> (String, usize);

    /// Full rebuild-migration SQL (FK control + BEGIN/COMMIT wrapper).
    fn rebuild_wrapper_sql(table: &str, temp: &str, inner_ddl: &str, indexes: &[String]) -> String;

    // ── Metadata queries (async) ────────────────────────────────

    /// Check if a table has a specific column.
    fn has_column(
        pool: &Pool,
        table: &str,
        column: &str,
    ) -> impl std::future::Future<Output = bool> + Send;

    /// Check if a table exists.
    fn table_exists(pool: &Pool, table: &str) -> impl std::future::Future<Output = bool> + Send;

    /// List user tables excluding core tables.
    fn list_user_tables(
        pool: &Pool,
        excluded: &str,
    ) -> impl std::future::Future<Output = Vec<String>> + Send;

    /// Fetch `(column_name, sql_type)` pairs for a table.
    fn fetch_columns_with_types(
        pool: &Pool,
        table: &str,
    ) -> impl std::future::Future<Output = Result<Vec<(String, String)>, sqlx::Error>> + Send;
}

// ════════════════════════════════════════════════════════════════
// SQLite
// ════════════════════════════════════════════════════════════════

#[cfg(feature = "db-sqlite")]
impl DbDriver for Sqlite {
    fn pk_type() -> &'static str {
        "INTEGER PRIMARY KEY"
    }

    fn fk_type() -> &'static str {
        "INTEGER"
    }

    fn ph(idx: usize) -> String {
        let _ = idx;
        "?".to_string()
    }

    fn now_fn() -> &'static str {
        "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')"
    }

    fn ago_expr(days: i64) -> String {
        format!("strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{days} days')")
    }

    fn date_trunc_day(col: &str) -> String {
        format!("DATE({col})")
    }

    fn current_date() -> String {
        "DATE('now')".to_string()
    }

    fn upsert_clause(conflict_cols: &str, assignments: &str) -> String {
        format!("ON CONFLICT({conflict_cols}) DO UPDATE SET {assignments}")
    }

    fn excluded_col(col: &str) -> String {
        format!("excluded.{col}")
    }

    fn cast_int(expr: &str) -> String {
        expr.to_string()
    }

    fn cast_ts(expr: &str) -> String {
        expr.to_string()
    }

    fn returning_clause() -> &'static str {
        "RETURNING *"
    }

    fn returning_col(col: &str) -> String {
        format!("RETURNING {col}")
    }

    fn insert_ignore_sql(table: &str, columns: &str, placeholders: &str) -> String {
        assert!(is_safe_identifier(table), "unsafe table name: {table}");
        format!("INSERT OR IGNORE INTO {table} ({columns}) VALUES ({placeholders})")
    }

    fn columns_sql(table: &str) -> (String, usize) {
        (format!("PRAGMA table_info({table})"), 1)
    }

    fn column_names_sql(table: &str) -> (String, usize) {
        (format!("PRAGMA table_info({table})"), 1)
    }

    fn rebuild_wrapper_sql(table: &str, temp: &str, inner_ddl: &str, indexes: &[String]) -> String {
        let index_sql = indexes
            .iter()
            .map(|i| format!("{i};\n"))
            .collect::<String>();
        let mut sql = String::new();
        sql.push_str("PRAGMA foreign_keys = OFF;\n\n");
        sql.push_str("BEGIN TRANSACTION;\n\n");
        sql.push_str(inner_ddl);
        sql.push_str(&format!("INSERT INTO {temp} SELECT * FROM {table};\n\n"));
        sql.push_str(&format!("DROP TABLE {table};\n\n"));
        sql.push_str(&format!("ALTER TABLE {temp} RENAME TO {table};\n\n"));
        sql.push_str(&index_sql);
        sql.push_str("\nPRAGMA foreign_key_check;\n\n");
        sql.push_str("COMMIT;\n\n");
        sql.push_str("PRAGMA foreign_keys = ON;\n");
        sql
    }

    async fn has_column(pool: &Pool, table: &str, column: &str) -> bool {
        assert!(is_safe_identifier(table), "unsafe table: {table}");
        let sql = format!("PRAGMA table_info({table})");
        let rows: Vec<(i32, String, String, bool, Option<String>, bool)> = sqlx::query_as(&sql)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        rows.iter().any(|(_, name, _, _, _, _)| name == column)
    }

    async fn table_exists(pool: &Pool, table: &str) -> bool {
        assert!(is_safe_identifier(table), "unsafe table: {table}");
        let sql =
            format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'");
        sqlx::query_scalar::<_, i64>(&sql)
            .fetch_one(pool)
            .await
            .unwrap_or(0)
            > 0
    }

    async fn list_user_tables(pool: &Pool, excluded: &str) -> Vec<String> {
        let sql = format!(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%%' AND name NOT IN ({excluded})"
        );
        let rows = sqlx::query_as::<_, (String,)>(&sql)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        rows.into_iter().map(|(n,)| n).collect()
    }

    async fn fetch_columns_with_types(
        pool: &Pool,
        table: &str,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        use sqlx::Row;
        let sql = format!("PRAGMA table_info({table})");
        let rows = sqlx::query(&sql).fetch_all(pool).await?;
        let mut cols = Vec::new();
        for row in &rows {
            let name: String = row.try_get(1).unwrap_or_default();
            let typ: String = row.try_get(2).unwrap_or_default();
            if !name.is_empty() {
                cols.push((name, typ));
            }
        }
        Ok(cols)
    }
}

// ════════════════════════════════════════════════════════════════
// PostgreSQL
// ════════════════════════════════════════════════════════════════

#[cfg(feature = "db-postgres")]
impl DbDriver for Postgres {
    fn pk_type() -> &'static str {
        "BIGINT PRIMARY KEY"
    }

    fn fk_type() -> &'static str {
        "BIGINT"
    }

    fn ph(idx: usize) -> String {
        format!("${idx}")
    }

    fn now_fn() -> &'static str {
        "NOW()"
    }

    fn ago_expr(days: i64) -> String {
        format!("NOW() - INTERVAL '{days} days'")
    }

    fn date_trunc_day(col: &str) -> String {
        format!("DATE_TRUNC('day', {col}::timestamp)")
    }

    fn current_date() -> String {
        "CURRENT_DATE".to_string()
    }

    fn upsert_clause(conflict_cols: &str, assignments: &str) -> String {
        format!("ON CONFLICT({conflict_cols}) DO UPDATE SET {assignments}")
    }

    fn excluded_col(col: &str) -> String {
        format!("excluded.{col}")
    }

    fn cast_int(expr: &str) -> String {
        expr.to_string()
    }

    fn cast_ts(expr: &str) -> String {
        expr.to_string()
    }

    fn returning_clause() -> &'static str {
        "RETURNING *"
    }

    fn returning_col(col: &str) -> String {
        format!("RETURNING {col}")
    }

    fn insert_ignore_sql(table: &str, columns: &str, placeholders: &str) -> String {
        assert!(is_safe_identifier(table), "unsafe table name: {table}");
        format!("INSERT INTO {table} ({columns}) VALUES ({placeholders}) ON CONFLICT DO NOTHING")
    }

    fn columns_sql(table: &str) -> (String, usize) {
        (
            format!(
                "SELECT column_name, data_type FROM information_schema.columns WHERE table_name = '{table}'"
            ),
            0,
        )
    }

    fn column_names_sql(table: &str) -> (String, usize) {
        (
            format!(
                "SELECT column_name FROM information_schema.columns WHERE table_name = '{table}'"
            ),
            0,
        )
    }

    fn rebuild_wrapper_sql(table: &str, temp: &str, inner_ddl: &str, indexes: &[String]) -> String {
        let index_sql = indexes
            .iter()
            .map(|i| format!("{i};\n"))
            .collect::<String>();
        let mut sql = String::new();
        sql.push_str("BEGIN;\n\n");
        sql.push_str(inner_ddl);
        sql.push_str(&format!("INSERT INTO {temp} SELECT * FROM {table};\n\n"));
        sql.push_str(&format!("DROP TABLE {table};\n\n"));
        sql.push_str(&format!("ALTER TABLE {temp} RENAME TO {table};\n\n"));
        sql.push_str(&index_sql);
        sql.push_str("\nCOMMIT;\n");
        sql
    }

    async fn has_column(pool: &Pool, table: &str, column: &str) -> bool {
        assert!(is_safe_identifier(table), "unsafe table: {table}");
        sqlx::query_scalar(
            "SELECT 1 FROM information_schema.columns WHERE table_name = $1 AND column_name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
        .is_some()
    }

    async fn table_exists(pool: &Pool, table: &str) -> bool {
        assert!(is_safe_identifier(table), "unsafe table: {table}");
        let sql = format!(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public' AND table_name = '{table}'"
        );
        sqlx::query_scalar::<_, i64>(&sql)
            .fetch_one(pool)
            .await
            .unwrap_or(0)
            > 0
    }

    async fn list_user_tables(pool: &Pool, excluded: &str) -> Vec<String> {
        let sql = format!(
            "SELECT tablename FROM pg_tables WHERE schemaname = 'public' AND tablename NOT IN ({excluded})"
        );
        let rows = sqlx::query_as::<_, (String,)>(&sql)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        rows.into_iter().map(|(n,)| n).collect()
    }

    async fn fetch_columns_with_types(
        pool: &Pool,
        table: &str,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        use sqlx::Row;
        let sql = format!(
            "SELECT column_name, data_type FROM information_schema.columns WHERE table_name = '{table}'"
        );
        let rows = sqlx::query(&sql).fetch_all(pool).await?;
        let mut cols = Vec::new();
        for row in &rows {
            let name: String = row.try_get(0).unwrap_or_default();
            let typ: String = row.try_get(1).unwrap_or_default();
            if !name.is_empty() {
                cols.push((name, typ));
            }
        }
        Ok(cols)
    }
}

// ════════════════════════════════════════════════════════════════
// MySQL
// ════════════════════════════════════════════════════════════════

#[cfg(feature = "db-mysql")]
impl DbDriver for MySql {
    fn pk_type() -> &'static str {
        "BIGINT PRIMARY KEY"
    }

    fn fk_type() -> &'static str {
        "BIGINT"
    }

    fn ph(idx: usize) -> String {
        let _ = idx;
        "?".to_string()
    }

    fn now_fn() -> &'static str {
        "NOW()"
    }

    fn ago_expr(days: i64) -> String {
        format!("DATE_SUB(NOW(), INTERVAL {days} DAY)")
    }

    fn date_trunc_day(col: &str) -> String {
        format!("CAST(DATE({col}) AS CHAR)")
    }

    fn current_date() -> String {
        "CURDATE()".to_string()
    }

    fn upsert_clause(conflict_cols: &str, assignments: &str) -> String {
        let _ = conflict_cols;
        format!("ON DUPLICATE KEY UPDATE {assignments}")
    }

    fn excluded_col(col: &str) -> String {
        format!("VALUES({col})")
    }

    fn cast_int(expr: &str) -> String {
        format!("CAST({expr} AS SIGNED)")
    }

    fn cast_ts(expr: &str) -> String {
        format!("CAST({expr} AS CHAR)")
    }

    fn returning_clause() -> &'static str {
        ""
    }

    fn returning_col(col: &str) -> String {
        let _ = col;
        String::new()
    }

    fn insert_ignore_sql(table: &str, columns: &str, placeholders: &str) -> String {
        assert!(is_safe_identifier(table), "unsafe table name: {table}");
        format!("INSERT IGNORE INTO {table} ({columns}) VALUES ({placeholders})")
    }

    fn columns_sql(table: &str) -> (String, usize) {
        (
            format!(
                "SELECT column_name, CAST(column_type AS CHAR) FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = '{table}'"
            ),
            0,
        )
    }

    fn column_names_sql(table: &str) -> (String, usize) {
        (
            format!(
                "SELECT column_name FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = '{table}'"
            ),
            0,
        )
    }

    fn rebuild_wrapper_sql(table: &str, temp: &str, inner_ddl: &str, indexes: &[String]) -> String {
        let index_sql = indexes
            .iter()
            .map(|i| format!("{i};\n"))
            .collect::<String>();
        let mut sql = String::new();
        sql.push_str("SET FOREIGN_KEY_CHECKS = 0;\n\n");
        sql.push_str(inner_ddl);
        sql.push_str(&format!("INSERT INTO {temp} SELECT * FROM {table};\n\n"));
        sql.push_str(&format!("DROP TABLE {table};\n\n"));
        sql.push_str(&format!("ALTER TABLE {temp} RENAME TO {table};\n\n"));
        sql.push_str(&index_sql);
        sql.push_str("\nSET FOREIGN_KEY_CHECKS = 1;\n");
        sql
    }

    async fn has_column(pool: &Pool, table: &str, column: &str) -> bool {
        assert!(is_safe_identifier(table), "unsafe table: {table}");
        sqlx::query_scalar::<crate::db::pool::Db, i32>(
            "SELECT 1 FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? AND column_name = ?",
        )
        .bind(table)
        .bind(column)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
        .is_some()
    }

    async fn table_exists(pool: &Pool, table: &str) -> bool {
        assert!(is_safe_identifier(table), "unsafe table: {table}");
        let sql = format!(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = '{table}'"
        );
        sqlx::query_scalar::<crate::db::pool::Db, i64>(&sql)
            .fetch_one(pool)
            .await
            .unwrap_or(0)
            > 0
    }

    async fn list_user_tables(pool: &Pool, excluded: &str) -> Vec<String> {
        let sql = format!(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name NOT IN ({excluded})"
        );
        let rows = sqlx::query_as::<crate::db::pool::Db, (String,)>(&sql)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        rows.into_iter().map(|(n,)| n).collect()
    }

    async fn fetch_columns_with_types(
        pool: &Pool,
        table: &str,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        use sqlx::Row;
        let sql = format!(
            "SELECT column_name, CAST(column_type AS CHAR) FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = '{table}'"
        );
        let rows = sqlx::query::<crate::db::pool::Db>(&sql)
            .fetch_all(pool)
            .await?;
        let mut cols = Vec::new();
        for row in &rows {
            let name: String = row.try_get(0).unwrap_or_default();
            let typ: String = row.try_get(1).unwrap_or_default();
            if !name.is_empty() {
                cols.push((name, typ));
            }
        }
        Ok(cols)
    }
}

// ════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_identifier_accepts_valid() {
        assert!(is_safe_identifier("users"));
        assert!(is_safe_identifier("created_at"));
        assert!(is_safe_identifier("_meta"));
        assert!(is_safe_identifier("col1"));
    }

    #[test]
    fn safe_identifier_rejects_invalid() {
        assert!(!is_safe_identifier(""));
        assert!(!is_safe_identifier("DROP TABLE"));
        assert!(!is_safe_identifier("id; DROP TABLE users--"));
        assert!(!is_safe_identifier("col name"));
        assert!(!is_safe_identifier("a'b"));
        assert!(!is_safe_identifier("1;DROP"));
        assert!(!is_safe_identifier(" posts "));
        assert!(!is_safe_identifier("posts "));
    }

    #[test]
    fn sanitize_identifier_trims_whitespace() {
        assert_eq!(sanitize_identifier("  posts  "), Some("posts"));
        assert_eq!(sanitize_identifier("users"), Some("users"));
        assert_eq!(sanitize_identifier("\t col1 \n"), Some("col1"));
        assert_eq!(sanitize_identifier("  "), None);
        assert_eq!(sanitize_identifier(" drop table "), None);
    }

    #[test]
    fn ph_generates_correct_placeholder() {
        let p = Driver::ph(1);
        #[cfg(feature = "db-sqlite")]
        assert_eq!(p, "?");
        #[cfg(feature = "db-postgres")]
        assert_eq!(p, "$1");
        #[cfg(feature = "db-mysql")]
        assert_eq!(p, "?");
    }

    #[test]
    fn pk_type_matches_backend() {
        let pk = Driver::pk_type();
        #[cfg(feature = "db-sqlite")]
        assert!(pk.starts_with("INTEGER"));
        #[cfg(any(feature = "db-postgres", feature = "db-mysql"))]
        assert!(pk.starts_with("BIGINT"));
    }
}

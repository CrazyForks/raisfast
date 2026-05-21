//! SQL dialect adaptation layer.
//!
//! Provides:
//! - [`ph`]: Generate placeholders for the current database (`?` or `$N`)
//! - [`is_safe_identifier`]: Validate identifiers contain only safe characters
//! - [`now_fn`] / [`ago_expr`] / [`date_trunc_day`]: Database dialect functions

/// Check if an identifier contains only safe characters (letters, digits, underscores).
///
/// Rejects empty strings and inputs containing spaces/special characters to prevent SQL injection.
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

/// Returns the SQL function for the current time in the current database.
///
/// - `SQLite`: `datetime('now')`
/// - `PostgreSQL` / `MySQL`: `NOW()`
#[must_use]
pub fn now_fn() -> &'static str {
    #[cfg(feature = "db-sqlite")]
    {
        "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')"
    }
    #[cfg(not(feature = "db-sqlite"))]
    {
        "NOW()"
    }
}

/// Returns a placeholder for the given positional index.
///
/// - `SQLite` / MySQL: `?`
/// - PostgreSQL: `$N`
///
/// ```ignore
/// let sql = format!("SELECT * FROM t WHERE id = {} AND name = {}", ph(1), ph(2));
/// ```
#[must_use]
pub fn ph(idx: usize) -> String {
    #[cfg(feature = "db-postgres")]
    {
        format!("${idx}")
    }
    #[cfg(not(feature = "db-postgres"))]
    {
        let _ = idx;
        "?".to_string()
    }
}

/// Returns a SQL expression for `N days ago` (for WHERE comparisons).
///
/// - SQLite: `datetime('now', '-{days} days')`
/// - PostgreSQL: `NOW() - INTERVAL '{days} days'`
/// - MySQL: `DATE_SUB(NOW(), INTERVAL {days} DAY)`
#[must_use]
pub fn ago_expr(days: i64) -> String {
    #[cfg(feature = "db-sqlite")]
    {
        format!("strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{days} days')")
    }
    #[cfg(feature = "db-postgres")]
    {
        format!("NOW() - INTERVAL '{days} days'")
    }
    #[cfg(feature = "db-mysql")]
    {
        format!("DATE_SUB(NOW(), INTERVAL {days} DAY)")
    }
}

/// Returns a SQL expression to truncate a date/time to day granularity.
///
/// - SQLite: `DATE({col})`
/// - PostgreSQL: `DATE_TRUNC('day', {col}::timestamp)`
/// - MySQL: `DATE({col})`
#[must_use]
pub fn date_trunc_day(col: &str) -> String {
    #[cfg(feature = "db-sqlite")]
    {
        format!("DATE({col})")
    }
    #[cfg(feature = "db-postgres")]
    {
        format!("DATE_TRUNC('day', {col}::timestamp)")
    }
    #[cfg(feature = "db-mysql")]
    {
        format!("DATE({col})")
    }
}

/// Returns an UPSERT conflict clause.
///
/// - SQLite / PostgreSQL: `ON CONFLICT({conflict_cols}) DO UPDATE SET {assignments}`
/// - MySQL: `ON DUPLICATE KEY UPDATE {assignments}`
#[must_use]
pub fn upsert_clause(conflict_cols: &str, assignments: &str) -> String {
    #[cfg(not(feature = "db-mysql"))]
    {
        format!("ON CONFLICT({conflict_cols}) DO UPDATE SET {assignments}")
    }
    #[cfg(feature = "db-mysql")]
    {
        let _ = conflict_cols;
        format!("ON DUPLICATE KEY UPDATE {assignments}")
    }
}

/// Returns an expression referencing the new value in an UPSERT.
///
/// - SQLite / PostgreSQL: `excluded.{col}`
/// - MySQL: `VALUES({col})`
#[must_use]
pub fn excluded_col(col: &str) -> String {
    #[cfg(not(feature = "db-mysql"))]
    {
        format!("excluded.{col}")
    }
    #[cfg(feature = "db-mysql")]
    {
        format!("VALUES({col})")
    }
}

/// Returns a `RETURNING *` clause (MySQL does not support RETURNING).
///
/// - SQLite / PostgreSQL: returns `RETURNING *`
/// - MySQL: returns empty string (requires a second query)
#[must_use]
pub fn returning_clause() -> &'static str {
    #[cfg(not(feature = "db-mysql"))]
    {
        "RETURNING *"
    }
    #[cfg(feature = "db-mysql")]
    {
        ""
    }
}

/// Returns a `RETURNING {col}` clause.
#[must_use]
pub fn returning_col(col: &str) -> String {
    #[cfg(not(feature = "db-mysql"))]
    {
        format!("RETURNING {col}")
    }
    #[cfg(feature = "db-mysql")]
    {
        let _ = col;
        String::new()
    }
}

/// Returns an INSERT IGNORE (or equivalent) SQL statement.
///
/// - SQLite: `INSERT OR IGNORE INTO {table} ({columns}) VALUES ({placeholders})`
/// - MySQL: `INSERT IGNORE INTO {table} ({columns}) VALUES ({placeholders})`
/// - PostgreSQL: `INSERT INTO {table} ({columns}) VALUES ({placeholders}) ON CONFLICT DO NOTHING`
#[must_use]
pub fn insert_ignore_sql(table: &str, columns: &str, placeholders: &str) -> String {
    assert!(is_safe_identifier(table), "unsafe table name: {table}");
    #[cfg(feature = "db-sqlite")]
    {
        format!("INSERT OR IGNORE INTO {table} ({columns}) VALUES ({placeholders})")
    }
    #[cfg(feature = "db-mysql")]
    {
        format!("INSERT IGNORE INTO {table} ({columns}) VALUES ({placeholders})")
    }
    #[cfg(feature = "db-postgres")]
    {
        format!("INSERT INTO {table} ({columns}) VALUES ({placeholders}) ON CONFLICT DO NOTHING")
    }
}

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
}

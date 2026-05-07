//! SQL 方言适配层。
//!
//! 提供：
//! - [`ph`]：生成当前数据库的占位符（`?` 或 `$N`）
//! - [`is_safe_identifier`]：校验标识符是否只含安全字符
//! - [`now_fn`] / [`ago_expr`] / [`date_trunc_day`]：数据库方言函数

/// 检查标识符是否只含安全字符（字母、数字、下划线），可用于表名、列名。
///
/// 拒绝空字符串和含空格/特殊字符的输入，防止 SQL 注入。
#[must_use]
pub fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// trim 后校验标识符。返回 `Some(trimmed)` 通过校验，`None` 不通过。
#[must_use]
pub fn sanitize_identifier(name: &str) -> Option<&str> {
    let trimmed = name.trim();
    if is_safe_identifier(trimmed) {
        Some(trimmed)
    } else {
        None
    }
}

/// 返回当前数据库获取当前时间的 SQL 函数。
///
/// - `SQLite`：`datetime('now')`
/// - `PostgreSQL` / `MySQL`：`NOW()`
#[must_use]
pub fn now_fn() -> &'static str {
    #[cfg(feature = "db-sqlite")]
    {
        "datetime('now')"
    }
    #[cfg(not(feature = "db-sqlite"))]
    {
        "NOW()"
    }
}

/// 返回指定位置序号的占位符。
///
/// - `SQLite` / MySQL：`?`
/// - PostgreSQL：`$N`
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

/// 返回 `N 天前` 的 SQL 表达式（用于 WHERE 比较）。
///
/// - SQLite: `datetime('now', '-{days} days')`
/// - PostgreSQL: `NOW() - INTERVAL '{days} days'`
/// - MySQL: `DATE_SUB(NOW(), INTERVAL {days} DAY)`
#[must_use]
pub fn ago_expr(days: i64) -> String {
    #[cfg(feature = "db-sqlite")]
    {
        format!("datetime('now', '-{days} days')")
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

/// 返回按天截断日期/时间的 SQL 表达式。
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

/// 返回 UPSERT 冲突子句。
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

/// 返回 UPSERT 中引用新值的表达式。
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

/// 返回 `RETURNING *` 子句（MySQL 不支持 RETURNING）。
///
/// - SQLite / PostgreSQL: 返回 `RETURNING *`（或空串 if不需要）
/// - MySQL: 返回空串（需二次查询）
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

/// 返回 `RETURNING {col}` 子句。
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

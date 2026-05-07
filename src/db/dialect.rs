//! SQL 方言适配层。
//!
//! 提供：
//! - [`translate`]：将 `?` 占位符翻译为 `PostgreSQL` 的 `$1, $2, ...` 格式
//! - [`now_fn`]：返回当前数据库的时间函数名（`datetime('now')` / `NOW()`）
//! - [`ago_expr`]：返回 `N 天前` 的 SQL 表达式
//! - [`date_trunc_day`]：返回按天截断的 SQL 表达式
//!
//! `SQLite` 和 `MySQL` 使用 `?` 占位符，无需翻译；
//! `PostgreSQL` 使用 `$N` 位置参数，需要运行时转换。

use std::borrow::Cow;

/// 将 SQL 中的 `?` 占位符翻译为目标数据库格式。
///
/// - `SQLite` / MySQL：原样返回
/// - `PostgreSQL`：`?` → `$1`, `$2`, ...
#[must_use]
pub fn translate(sql: &str) -> Cow<'_, str> {
    #[cfg(not(feature = "db-postgres"))]
    {
        let _ = sql;
    }

    #[cfg(feature = "db-postgres")]
    {
        if !sql.contains('?') {
            return Cow::Borrowed(sql);
        }
        let mut result = String::with_capacity(sql.len() + 16);
        let mut n: usize = 0;
        for ch in sql.chars() {
            if ch == '?' {
                n += 1;
                use std::fmt::Write;
                write!(result, "${n}").unwrap();
            } else {
                result.push(ch);
            }
        }
        return Cow::Owned(result);
    }

    #[cfg(not(feature = "db-postgres"))]
    Cow::Borrowed(sql)
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
#[must_use]
pub fn translate_placeholder(idx: usize) -> String {
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
    fn translate_no_placeholders() {
        assert_eq!(translate("SELECT 1"), "SELECT 1");
    }

    #[test]
    #[cfg(not(feature = "db-postgres"))]
    fn translate_keeps_question_marks() {
        assert_eq!(
            translate("SELECT * FROM users WHERE id = ?"),
            "SELECT * FROM users WHERE id = ?"
        );
    }

    #[test]
    #[cfg(feature = "db-postgres")]
    fn translate_converts_to_positional() {
        assert_eq!(
            translate("SELECT * FROM users WHERE id = ? AND name = ?"),
            "SELECT * FROM users WHERE id = $1 AND name = $2"
        );
    }
}

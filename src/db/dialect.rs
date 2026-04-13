//! SQL 方言适配层。
//!
//! 提供：
//! - [`translate`]：将 `?` 占位符翻译为 PostgreSQL 的 `$1, $2, ...` 格式
//! - [`now_fn`]：返回当前数据库的时间函数名（`datetime('now')` / `NOW()`）
//!
//! SQLite 和 MySQL 使用 `?` 占位符，无需翻译；
//! PostgreSQL 使用 `$N` 位置参数，需要运行时转换。
//! `sqlx::query!` 宏会自动处理占位符，但 `sqlx::query()` /
//! `sqlx::query_as()` 等运行时调用需手动翻译。

use std::borrow::Cow;

/// 将 SQL 中的 `?` 占位符翻译为目标数据库格式。
///
/// - SQLite / MySQL：原样返回
/// - PostgreSQL：`?` → `$1`, `$2`, ...
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
/// - SQLite：`datetime('now')`
/// - PostgreSQL / MySQL：`NOW()`
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

//! 租户感知的数据库操作层
//!
//! `TenantPool` 包装 `Pool`，为所有 SQL 操作自动检测并注入 `tenant_id`：
//! - 表有 `tenant_id` 列 → 自动追加 `AND tenant_id = ?` 并绑定
//! - 表无 `tenant_id` 列 → 查询不受影响
//!
//! # 使用方式
//!
//! ```ignore
//! let tp = TenantPool::new(pool, "default");
//!
//! // SELECT — 返回改写后的 SQL + 是否需要额外 bind tenant_id
//! let (sql, bind_tenant) = tp.prepare_select("posts", "SELECT * FROM posts WHERE id = ?").await;
//! let mut q = sqlx::query_as::<_, Post>(&sql).bind(id);
//! if bind_tenant { q = q.bind(tp.tenant_id()); }
//! let post = q.fetch_optional(tp.pool()).await?;
//!
//! // INSERT — 返回改写后的 SQL（自动追加 tenant_id 列）
//! let (sql, bind_tenant) = tp.prepare_insert("posts", "title, slug", 2).await;
//! let mut q = sqlx::query(&sql).bind(title).bind(slug);
//! if bind_tenant { q = q.bind(tp.tenant_id()); }
//! q.execute(tp.pool()).await?;
//!
//! // UPDATE/DELETE — 返回改写后的 SQL + 是否需要额外 bind
//! let (sql, bind_tenant) = tp.prepare_modify("posts", "DELETE FROM posts WHERE id = ?").await;
//! let mut q = sqlx::query(&sql).bind(id);
//! if bind_tenant { q = q.bind(tp.tenant_id()); }
//! q.execute(tp.pool()).await?;
//! ```

use std::collections::HashSet;

use tokio::sync::RwLock;

use super::Pool;

// ---------------------------------------------------------------------------
// 列检测缓存
// ---------------------------------------------------------------------------

static CACHE: std::sync::OnceLock<RwLock<HashSet<String>>> = std::sync::OnceLock::new();

fn cache() -> &'static RwLock<HashSet<String>> {
    CACHE.get_or_init(|| RwLock::new(HashSet::new()))
}

/// 检测指定表是否包含 `tenant_id` 列（带全局缓存，首次后零开销）。
pub async fn has_tenant_id(pool: &Pool, table: &str) -> bool {
    {
        let r = cache().read().await;
        if r.contains(table) {
            return true;
        }
    }
    let exists = check_column_exists(pool, table).await;
    if exists {
        cache().write().await.insert(table.to_string());
    }
    exists
}

/// 清除缓存（测试用）。
pub async fn invalidate_cache() {
    cache().write().await.clear();
}

async fn check_column_exists(pool: &Pool, table: &str) -> bool {
    assert!(
        super::dialect::is_safe_identifier(table),
        "unsafe table name: {table}"
    );
    #[cfg(feature = "db-sqlite")]
    {
        let sql = format!("PRAGMA table_info({table})");
        let rows: Vec<(i32, String, String, i32, Option<String>, i32)> = sqlx::query_as(&sql)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        rows.iter().any(|(_, name, _, _, _, _)| name == "tenant_id")
    }
    #[cfg(feature = "db-postgres")]
    {
        sqlx::query_scalar("SELECT 1 FROM information_schema.columns WHERE table_name = $1 AND column_name = 'tenant_id'")
            .bind(table)
            .fetch_optional(pool)
            .await
            .unwrap_or(None)
            .is_some()
    }
    #[cfg(feature = "db-mysql")]
    {
        sqlx::query_scalar("SELECT 1 FROM information_schema.columns WHERE table_name = ? AND column_name = 'tenant_id'")
            .bind(table)
            .fetch_optional(pool)
            .await
            .unwrap_or(None)
            .is_some()
    }
}

// ---------------------------------------------------------------------------
// SQL 改写
// ---------------------------------------------------------------------------

fn count_params(sql: &str) -> usize {
    #[cfg(feature = "db-postgres")]
    {
        let mut max_n = 0;
        let bytes = sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if let Ok(n) = sql[start..j].parse::<usize>() {
                    max_n = max_n.max(n);
                }
                i = j;
            } else {
                i += 1;
            }
        }
        max_n
    }
    #[cfg(not(feature = "db-postgres"))]
    {
        sql.matches('?').count()
    }
}

fn inject_where(sql: &str, idx: usize) -> String {
    let connector = if sql.to_lowercase().contains("where") {
        " AND "
    } else {
        " WHERE "
    };
    format!("{sql}{connector}tenant_id = {}", super::dialect::ph(idx))
}

/// 解析 `Option<&str>` 为有效的租户 ID。
///
/// `None`（超管未指定租户）回退到 [`DEFAULT_TENANT`]。
/// 用于 INSERT 等必须有值的场景。
pub fn resolve_tenant(tenant_id: Option<&str>) -> &str {
    tenant_id.unwrap_or(crate::constants::DEFAULT_TENANT)
}

fn sql_has_tenant(sql: &str) -> bool {
    sql.to_lowercase().contains("tenant_id")
}

/// 返回 `AND tenant_id = {ph(idx)}` 或空串，用于条件 SQL 拼接。
///
/// ```ignore
/// let sql = format!("SELECT * FROM users WHERE id = {}{}", ph(1), tenant_filter_ph(tenant_id, 2));
/// ```
pub fn tenant_filter_ph(tenant_id: Option<&str>, idx: usize) -> String {
    match tenant_id {
        Some(_) => format!(" AND tenant_id = {}", super::dialect::ph(idx)),
        None => String::new(),
    }
}

/// 返回 ` And p.tenant_id = ?` 或空串，用于 JOIN 查询中带表别名的条件拼接。
pub fn tenant_filter_aliased(alias: &str, tenant_id: Option<&str>) -> String {
    match tenant_id {
        Some(_) => format!(" AND {alias}.tenant_id = ?"),
        None => String::new(),
    }
}

/// [`tenant_filter_aliased`] 的占位符安全版本。
pub fn tenant_filter_aliased_ph(alias: &str, tenant_id: Option<&str>, idx: usize) -> String {
    match tenant_id {
        Some(_) => format!(" AND {alias}.tenant_id = {}", super::dialect::ph(idx)),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// TenantPool
// ---------------------------------------------------------------------------

/// 租户感知的数据库连接。
///
/// 核心流程：**检测列 → 改写 SQL → 用户 bind → 自动 bind tenant_id → 执行**。
#[derive(Clone)]
pub struct TenantPool {
    pool: Pool,
    tenant_id: String,
}

impl TenantPool {
    /// 创建 `TenantPool`。
    pub fn new(pool: Pool, tenant_id: impl Into<String>) -> Self {
        Self {
            pool,
            tenant_id: tenant_id.into(),
        }
    }

    /// 内部连接池。
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// 当前租户 ID。
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// **SELECT 预处理**：检测表是否有 `tenant_id` 列，改写 SQL。
    ///
    /// 返回 `(final_sql, bind_tenant)`：
    /// - `final_sql`：改写后的 SQL（可能追加了 `AND tenant_id = ?`）
    /// - `bind_tenant`：是否需要在用户参数之后额外 `.bind(tenant_id)`
    pub async fn prepare_select(&self, table: &str, sql: &str) -> (String, bool) {
        let has = has_tenant_id(&self.pool, table).await;
        let inject = has && !sql_has_tenant(sql);
        let final_sql = if inject {
            inject_where(sql, count_params(sql) + 1)
        } else {
            sql.to_string()
        };
        (final_sql, inject)
    }

    /// **INSERT 预处理**：检测表，构建 INSERT SQL（自动追加 `tenant_id` 列）。
    ///
    /// `user_cols` — 列名（逗号分隔，不含 `tenant_id`）。
    /// `user_param_count` — 用户参数数量。
    ///
    /// 返回 `(final_sql, bind_tenant)`。
    pub async fn prepare_insert(
        &self,
        table: &str,
        user_cols: &str,
        user_param_count: usize,
    ) -> (String, bool) {
        let has = has_tenant_id(&self.pool, table).await;
        let (cols, placeholders) = if has {
            let placeholders: Vec<String> =
                (1..=user_param_count + 1).map(super::dialect::ph).collect();
            (format!("{user_cols}, tenant_id"), placeholders.join(", "))
        } else {
            let placeholders: Vec<String> =
                (1..=user_param_count).map(super::dialect::ph).collect();
            (user_cols.to_string(), placeholders.join(", "))
        };
        let sql = format!("INSERT INTO {table} ({cols}) VALUES ({placeholders})");
        (sql, has)
    }

    /// **UPDATE/DELETE 预处理**：检测表，改写 SQL（追加 `AND tenant_id = ?`）。
    ///
    /// 返回 `(final_sql, bind_tenant)`。
    pub async fn prepare_modify(&self, table: &str, sql: &str) -> (String, bool) {
        let has = has_tenant_id(&self.pool, table).await;
        let inject = has && !sql_has_tenant(sql);
        let final_sql = if inject {
            inject_where(sql, count_params(sql) + 1)
        } else {
            sql.to_string()
        };
        (final_sql, inject)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_where_with_existing() {
        assert_eq!(
            inject_where("SELECT * FROM posts WHERE id = ?", 2),
            "SELECT * FROM posts WHERE id = ? AND tenant_id = ?"
        );
    }

    #[test]
    fn inject_where_without_existing() {
        assert_eq!(
            inject_where("SELECT * FROM posts", 1),
            "SELECT * FROM posts WHERE tenant_id = ?"
        );
    }

    #[tokio::test]
    async fn prepare_select_injects_when_has_column() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE posts (id TEXT, title TEXT, tenant_id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        invalidate_cache().await;

        let tp = TenantPool::new(pool, "t1");
        let (sql, bind) = tp
            .prepare_select("posts", "SELECT * FROM posts WHERE id = ?")
            .await;
        assert!(bind);
        assert!(sql.contains("tenant_id"));
    }

    #[tokio::test]
    async fn prepare_select_skips_when_no_column() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE logs (id TEXT, msg TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        invalidate_cache().await;

        let tp = TenantPool::new(pool, "t1");
        let (sql, bind) = tp
            .prepare_select("logs", "SELECT * FROM logs WHERE id = ?")
            .await;
        assert!(!bind);
        assert!(!sql.contains("tenant_id"));
    }

    #[tokio::test]
    async fn prepare_insert_injects_column() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE items (id TEXT, name TEXT, tenant_id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        invalidate_cache().await;

        let tp = TenantPool::new(pool, "t1");
        let (sql, bind) = tp.prepare_insert("items", "id, name", 2).await;
        assert!(bind);
        assert!(sql.contains("tenant_id"));
        assert!(sql.contains("?, ?")); // 2 user + 1 tenant
    }

    #[tokio::test]
    async fn end_to_end_select_filters_by_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE posts (id TEXT, title TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO posts (id, title, tenant_id) VALUES ('1', 'Hello', 't1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO posts (id, title, tenant_id) VALUES ('2', 'World', 't2')")
            .execute(&pool)
            .await
            .unwrap();
        invalidate_cache().await;

        #[derive(sqlx::FromRow)]
        #[allow(dead_code)]
        struct Post {
            id: String,
            title: String,
        }

        let tp = TenantPool::new(pool, "t1");
        let (sql, bind) = tp
            .prepare_select("posts", "SELECT id, title FROM posts WHERE id = ?")
            .await;
        let mut q = sqlx::query_as::<_, Post>(&sql).bind("1");
        if bind {
            q = q.bind(tp.tenant_id());
        }
        let p: Post = q.fetch_one(tp.pool()).await.unwrap();
        assert_eq!(p.title, "Hello");

        let (sql, bind) = tp
            .prepare_select("posts", "SELECT id, title FROM posts WHERE id = ?")
            .await;
        let mut q = sqlx::query_as::<_, Post>(&sql).bind("2");
        if bind {
            q = q.bind(tp.tenant_id());
        }
        assert!(q.fetch_optional(tp.pool()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn end_to_end_insert_auto_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE items (id TEXT, name TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();
        invalidate_cache().await;

        let tp = TenantPool::new(pool.clone(), "t1");
        let (sql, bind) = tp.prepare_insert("items", "id, name", 2).await;
        let mut q = sqlx::query(&sql).bind("i1").bind("Test");
        if bind {
            q = q.bind(tp.tenant_id());
        }
        q.execute(tp.pool()).await.unwrap();

        let row: (String, String, String) = sqlx::query_as("SELECT id, name, tenant_id FROM items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.2, "t1");
    }

    #[tokio::test]
    async fn end_to_end_delete_respects_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE items (id TEXT, name TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO items VALUES ('1', 'A', 't1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO items VALUES ('2', 'B', 't2')")
            .execute(&pool)
            .await
            .unwrap();
        invalidate_cache().await;

        let tp = TenantPool::new(pool.clone(), "t1");

        // 删除其他租户的数据 → 不影响
        let (sql, bind) = tp
            .prepare_modify("items", "DELETE FROM items WHERE id = ?")
            .await;
        let mut q = sqlx::query(&sql).bind("2");
        if bind {
            q = q.bind(tp.tenant_id());
        }
        q.execute(tp.pool()).await.unwrap();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2);

        // 删除自己的数据 → 成功
        let (sql, bind) = tp
            .prepare_modify("items", "DELETE FROM items WHERE id = ?")
            .await;
        let mut q = sqlx::query(&sql).bind("1");
        if bind {
            q = q.bind(tp.tenant_id());
        }
        q.execute(tp.pool()).await.unwrap();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }
}

//! Tenant-aware database operations layer
//!
//! `TenantPool` wraps `Pool` to automatically detect and inject `tenant_id` for all SQL operations:
//! - Table has `tenant_id` column → automatically appends `AND tenant_id = ?` and binds
//! - Table has no `tenant_id` column → query is unaffected
//!
//! # Usage
//!
//! ```ignore
//! let tp = TenantPool::new(pool, "default");
//!
//! // SELECT — returns rewritten SQL + whether tenant_id needs to be bound
//! let (sql, bind_tenant) = tp.prepare_select("posts", "SELECT * FROM posts WHERE id = ?").await;
//! let mut q = sqlx::query_as::<_, Post>(&sql).bind(id);
//! if bind_tenant { q = q.bind(tp.tenant_id()); }
//! let post = q.fetch_optional(tp.pool()).await?;
//!
//! // INSERT — returns rewritten SQL (automatically appends tenant_id column)
//! let (sql, bind_tenant) = tp.prepare_insert("posts", "title, slug", 2).await;
//! let mut q = sqlx::query(&sql).bind(title).bind(slug);
//! if bind_tenant { q = q.bind(tp.tenant_id()); }
//! q.execute(tp.pool()).await?;
//!
//! // UPDATE/DELETE — returns rewritten SQL + whether extra bind is needed
//! let (sql, bind_tenant) = tp.prepare_modify("posts", "DELETE FROM posts WHERE id = ?").await;
//! let mut q = sqlx::query(&sql).bind(id);
//! if bind_tenant { q = q.bind(tp.tenant_id()); }
//! q.execute(tp.pool()).await?;
//! ```

use super::DbDriver;
use std::collections::HashSet;

use tokio::sync::RwLock;

use crate::constants::COL_TENANT_ID;

use super::Pool;

// ---------------------------------------------------------------------------
// Column detection cache
// ---------------------------------------------------------------------------

static CACHE: std::sync::OnceLock<RwLock<HashSet<String>>> = std::sync::OnceLock::new();

fn cache() -> &'static RwLock<HashSet<String>> {
    CACHE.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Check if the specified table has a `tenant_id` column (with global cache, zero overhead after first check).
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

/// Clear the cache (for testing).
pub async fn invalidate_cache() {
    cache().write().await.clear();
}

async fn check_column_exists(pool: &Pool, table: &str) -> bool {
    assert!(
        super::driver::is_safe_identifier(table),
        "unsafe table name: {table}"
    );
    super::Driver::has_column(pool, table, COL_TENANT_ID).await
}

// ---------------------------------------------------------------------------
// SQL rewriting
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
    format!(
        "{sql}{connector}{COL_TENANT_ID} = {}",
        super::Driver::ph(idx)
    )
}

/// Resolve `Option<&str>` to a valid tenant ID.
///
/// `None` (superadmin without a specified tenant) falls back to [`DEFAULT_TENANT`].
/// Used for INSERT and other scenarios where a value is required.
pub fn resolve_tenant(tenant_id: Option<&str>) -> &str {
    tenant_id.unwrap_or(crate::constants::DEFAULT_TENANT)
}

fn sql_has_tenant(sql: &str) -> bool {
    sql.to_lowercase().contains(COL_TENANT_ID)
}

/// Returns `AND tenant_id = {ph(idx)}` or empty string, for conditional SQL concatenation.
///
/// ```ignore
/// let sql = format!("SELECT * FROM users WHERE id = {}{}", ph(1), tenant_filter_ph(tenant_id, 2));
/// ```
pub fn tenant_filter_ph(tenant_id: Option<&str>, idx: usize) -> String {
    match tenant_id {
        Some(_) => format!(" AND {COL_TENANT_ID} = {}", super::Driver::ph(idx)),
        None => String::new(),
    }
}

/// Returns ` And p.tenant_id = ?` or empty string, for JOIN query conditional concatenation with table alias.
pub fn tenant_filter_aliased(alias: &str, tenant_id: Option<&str>) -> String {
    match tenant_id {
        Some(_) => format!(" AND {alias}.{COL_TENANT_ID} = ?"),
        None => String::new(),
    }
}

/// Build an INSERT SQL statement with optional `tenant_id` column.
///
/// When `tenant_id` is `Some`, appends `tenant_id` to the column list
/// and adds one more placeholder. The caller must call `bind_tenant!`
/// after binding all user columns.
///
/// # Example
///
/// ```ignore
/// let sql = insert_sql("tags", &["id", "name", "slug"], tenant_id);
/// let mut q = sqlx::query(&sql).bind(&id).bind(name).bind(slug);
/// bind_tenant!(q, tenant_id);
/// q.execute(pool).await?;
/// ```
pub fn insert_sql(table: &str, columns: &[&str], tenant_id: Option<&str>) -> String {
    assert!(
        super::driver::is_safe_identifier(table),
        "unsafe table name: {table}"
    );
    let mut cols: Vec<&str> = columns.to_vec();
    if tenant_id.is_some() {
        cols.push(COL_TENANT_ID);
    }
    let phs: Vec<String> = (1..=cols.len()).map(super::Driver::ph).collect();
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        cols.join(", "),
        phs.join(", ")
    )
}

/// Placeholder-safe version of [`tenant_filter_aliased`].
pub fn tenant_filter_aliased_ph(alias: &str, tenant_id: Option<&str>, idx: usize) -> String {
    match tenant_id {
        Some(_) => format!(" AND {alias}.{COL_TENANT_ID} = {}", super::Driver::ph(idx)),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// TenantPool
// ---------------------------------------------------------------------------

/// Tenant-aware database connection.
///
/// Core flow: **detect column → rewrite SQL → user bind → auto bind tenant_id → execute**.
#[derive(Clone)]
pub struct TenantPool {
    pool: Pool,
    tenant_id: String,
}

impl TenantPool {
    /// Create a `TenantPool`.
    pub fn new(pool: Pool, tenant_id: impl Into<String>) -> Self {
        Self {
            pool,
            tenant_id: tenant_id.into(),
        }
    }

    /// Inner connection pool.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Current tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// **SELECT preprocessing**: detect if table has `tenant_id` column, rewrite SQL.
    ///
    /// Returns `(final_sql, bind_tenant)`:
    /// - `final_sql`: rewritten SQL (may have `AND tenant_id = ?` appended)
    /// - `bind_tenant`: whether `.bind(tenant_id)` needs to be called after user params
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

    /// **INSERT preprocessing**: detect table, build INSERT SQL (automatically appends `tenant_id` column).
    ///
    /// `user_cols` — column names (comma-separated, excluding `tenant_id`).
    /// `user_param_count` — number of user parameters.
    ///
    /// Returns `(final_sql, bind_tenant)`.
    pub async fn prepare_insert(
        &self,
        table: &str,
        user_cols: &str,
        user_param_count: usize,
    ) -> (String, bool) {
        let has = has_tenant_id(&self.pool, table).await;
        let (cols, placeholders) = if has {
            let placeholders: Vec<String> =
                (1..=user_param_count + 1).map(super::Driver::ph).collect();
            (
                format!("{user_cols}, {COL_TENANT_ID}"),
                placeholders.join(", "),
            )
        } else {
            let placeholders: Vec<String> = (1..=user_param_count).map(super::Driver::ph).collect();
            (user_cols.to_string(), placeholders.join(", "))
        };
        let sql = format!("INSERT INTO {table} ({cols}) VALUES ({placeholders})");
        (sql, has)
    }

    /// **UPDATE/DELETE preprocessing**: detect table, rewrite SQL (append `AND tenant_id = ?`).
    ///
    /// Returns `(final_sql, bind_tenant)`.
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
        sqlx::query(
            "CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, tenant_id TEXT)",
        )
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
        sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT, msg TEXT)")
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
        sqlx::query(
            "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, tenant_id TEXT)",
        )
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
            "CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO posts (id, title, tenant_id) VALUES (1, 'Hello', 't1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO posts (id, title, tenant_id) VALUES (2, 'World', 't2')")
            .execute(&pool)
            .await
            .unwrap();
        invalidate_cache().await;

        #[derive(sqlx::FromRow)]
        #[allow(dead_code)]
        struct Post {
            title: String,
        }

        let tp = TenantPool::new(pool, "t1");
        let (sql, bind) = tp
            .prepare_select("posts", "SELECT title FROM posts WHERE id = ?")
            .await;
        let mut q = sqlx::query_as::<_, Post>(&sql).bind(1i64);
        if bind {
            q = q.bind(tp.tenant_id());
        }
        let p: Post = q.fetch_one(tp.pool()).await.unwrap();
        assert_eq!(p.title, "Hello");

        let (sql, bind) = tp
            .prepare_select("posts", "SELECT title FROM posts WHERE id = ?")
            .await;
        let mut q = sqlx::query_as::<_, Post>(&sql).bind(2i64);
        if bind {
            q = q.bind(tp.tenant_id());
        }
        assert!(q.fetch_optional(tp.pool()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn end_to_end_insert_auto_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();
        invalidate_cache().await;

        let tp = TenantPool::new(pool.clone(), "t1");
        let (sql, bind) = tp.prepare_insert("items", "id, name", 2).await;
        let mut q = sqlx::query(&sql).bind(1i64).bind("Test");
        if bind {
            q = q.bind(tp.tenant_id());
        }
        q.execute(tp.pool()).await.unwrap();

        let row: (i64, String, String) = sqlx::query_as("SELECT id, name, tenant_id FROM items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.2, "t1");
    }

    #[tokio::test]
    async fn end_to_end_delete_respects_tenant() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO items (id, name, tenant_id) VALUES (1, 'A', 't1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO items (id, name, tenant_id) VALUES (2, 'B', 't2')")
            .execute(&pool)
            .await
            .unwrap();
        invalidate_cache().await;

        let tp = TenantPool::new(pool.clone(), "t1");

        let (sql, bind) = tp
            .prepare_modify("items", "DELETE FROM items WHERE id = ?")
            .await;
        let mut q = sqlx::query(&sql).bind(2i64);
        if bind {
            q = q.bind(tp.tenant_id());
        }
        q.execute(tp.pool()).await.unwrap();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2);

        let (sql, bind) = tp
            .prepare_modify("items", "DELETE FROM items WHERE id = ?")
            .await;
        let mut q = sqlx::query(&sql).bind(1i64);
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

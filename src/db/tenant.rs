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

fn inject_where(sql: &str) -> String {
    let connector = if sql.to_lowercase().contains("where") {
        " AND "
    } else {
        " WHERE "
    };
    format!("{sql}{connector}tenant_id = ?")
}

/// 默认租户 ID。
pub const DEFAULT_TENANT: &str = "default";

/// 解析 `Option<&str>` 为有效的租户 ID。
///
/// `None`（超管未指定租户）回退到 [`DEFAULT_TENANT`]。
/// 用于 INSERT 等必须有值的场景。
pub fn resolve_tenant(tenant_id: Option<&str>) -> &str {
    tenant_id.unwrap_or(DEFAULT_TENANT)
}

fn sql_has_tenant(sql: &str) -> bool {
    sql.to_lowercase().contains("tenant_id")
}

/// 返回 `AND tenant_id = ?` 或空串，用于静态表的条件 SQL 拼接。
///
/// ```ignore
/// let sql = format!("SELECT * FROM users WHERE id = ?{}", tenant_filter(tenant_id));
/// ```
pub fn tenant_filter(tenant_id: Option<&str>) -> &'static str {
    match tenant_id {
        Some(_) => " AND tenant_id = ?",
        None => "",
    }
}

/// 返回 ` AND p.tenant_id = ?` 或空串，用于 JOIN 查询中带表别名的条件拼接。
pub fn tenant_filter_aliased(alias: &str, tenant_id: Option<&str>) -> String {
    match tenant_id {
        Some(_) => format!(" AND {alias}.tenant_id = ?"),
        None => String::new(),
    }
}

/// 为 SQL 追加 `AND tenant_id = ?`（当 `tenant_id` 不为 `None` 时）。
///
/// 若 `tenant_id` 为 `None`（超管看所有），返回原始 SQL（需 `Cow`）。
/// 若 SQL 已包含 `tenant_id`，不重复追加。
pub fn append_tenant_filter<'a>(
    sql: &'a str,
    tenant_id: Option<&str>,
) -> std::borrow::Cow<'a, str> {
    match tenant_id {
        Some(_) if !sql_has_tenant(sql) => {
            let connector = if sql.to_lowercase().contains("where") {
                " AND tenant_id = ?"
            } else {
                " WHERE tenant_id = ?"
            };
            std::borrow::Cow::Owned(format!("{sql}{connector}"))
        }
        _ => std::borrow::Cow::Borrowed(sql),
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
            let rewritten = inject_where(sql);
            super::dialect::translate(&rewritten).into_owned()
        } else {
            super::dialect::translate(sql).into_owned()
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
            let mut ph: Vec<&str> = (0..user_param_count).map(|_| "?").collect();
            ph.push("?");
            (format!("{user_cols}, tenant_id"), ph.join(", "))
        } else {
            let ph: Vec<&str> = (0..user_param_count).map(|_| "?").collect();
            (user_cols.to_string(), ph.join(", "))
        };
        let sql = format!("INSERT INTO {table} ({cols}) VALUES ({placeholders})");
        (super::dialect::translate(&sql).into_owned(), has)
    }

    /// **UPDATE/DELETE 预处理**：检测表，改写 SQL（追加 `AND tenant_id = ?`）。
    ///
    /// 返回 `(final_sql, bind_tenant)`。
    pub async fn prepare_modify(&self, table: &str, sql: &str) -> (String, bool) {
        let has = has_tenant_id(&self.pool, table).await;
        let inject = has && !sql_has_tenant(sql);
        let final_sql = if inject {
            let rewritten = inject_where(sql);
            super::dialect::translate(&rewritten).into_owned()
        } else {
            super::dialect::translate(sql).into_owned()
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
            inject_where("SELECT * FROM posts WHERE id = ?"),
            "SELECT * FROM posts WHERE id = ? AND tenant_id = ?"
        );
    }

    #[test]
    fn inject_where_without_existing() {
        assert_eq!(
            inject_where("SELECT * FROM posts"),
            "SELECT * FROM posts WHERE tenant_id = ?"
        );
    }

    #[tokio::test]
    async fn prepare_select_injects_when_has_column() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
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
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
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
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
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
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
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
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
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
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
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

//! Dashboard statistics service.
//!
//! Provides aggregated statistics for the admin dashboard:
//! - Overview (total counts per entity, content type distribution, recent activity)
//! - Per-content-type statistics (status distribution)
//! - Trend data (daily creation counts over the last N days)

use serde_json::{Value, json};

use crate::db::DbDriver;
use crate::db::Pool;
use crate::errors::app_error::AppError;

/// Dashboard statistics service
pub struct StatsService {
    pool: Pool,
}

impl StatsService {
    /// Create a new statistics service instance
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Overview statistics.
    ///
    /// Returns total counts per entity, content type distribution, and recent activity.
    pub async fn overview(&self, tenant_id: Option<&str>) -> Result<Value, AppError> {
        let tf = crate::db::tenant::tenant_filter_ph(tenant_id, 1);
        let tf_aliased = crate::db::tenant::tenant_filter_aliased_ph("p", tenant_id, 1);

        let total_posts = count_table(&self.pool, "posts", &tf_aliased, tenant_id).await?;
        let total_comments = count_table(&self.pool, "comments", &tf_aliased, tenant_id).await?;
        let total_users = count_table(&self.pool, "users", &tf, tenant_id).await?;
        let total_media = count_table(&self.pool, "media", &tf, tenant_id).await?;
        let total_categories =
            count_table(&self.pool, "categories", &tf_aliased, tenant_id).await?;
        let total_tags = count_table(&self.pool, "tags", &tf_aliased, tenant_id).await?;

        let total_products = count_table(&self.pool, "products", &tf, tenant_id).await?;
        let total_orders = count_table(&self.pool, "orders", &tf, tenant_id).await?;
        let total_coupons = count_table(&self.pool, "coupons", &tf, tenant_id).await?;

        let products_by_status = self.count_by_status("products", tenant_id).await?;
        let orders_by_status = self.count_by_status("orders", tenant_id).await?;

        let total_revenue = self.sum_revenue(tenant_id).await?;

        let content_by_type = self.count_content_types(tenant_id).await?;

        let posts_by_status = self.count_by_status("posts", tenant_id).await?;
        let comments_by_status = self.count_by_status("comments", tenant_id).await?;

        let recent_activity = self.recent_activity(tenant_id, 10).await?;

        Ok(json!({
            "total_posts": total_posts,
            "total_comments": total_comments,
            "total_users": total_users,
            "total_media": total_media,
            "total_categories": total_categories,
            "total_tags": total_tags,
            "total_products": total_products,
            "total_orders": total_orders,
            "total_coupons": total_coupons,
            "total_revenue": total_revenue,
            "posts_by_status": posts_by_status,
            "comments_by_status": comments_by_status,
            "products_by_status": products_by_status,
            "orders_by_status": orders_by_status,
            "content_by_type": content_by_type,
            "recent_activity": recent_activity,
        }))
    }

    /// Per-content-type statistics (status distribution)
    pub async fn content_stats(
        &self,
        table: &str,
        tenant_id: Option<&str>,
    ) -> Result<Value, AppError> {
        validate_table_name(table)?;
        let tf = crate::db::tenant::tenant_filter_ph(tenant_id, 1);

        let has_status = has_column(&self.pool, table, "status").await;
        let has_tenant = crate::db::tenant::has_tenant_id(&self.pool, table).await;

        let total = count_table(&self.pool, table, &tf, tenant_id).await?;

        let mut result = json!({
            "table": table,
            "total": total,
        });

        if has_status {
            let cnt_expr = crate::db::Driver::cast_int("COUNT(*)");
            let status_sql = if has_tenant {
                let tid = crate::db::tenant::resolve_tenant(tenant_id).to_string();
                let sql = format!(
                    "SELECT status, {cnt_expr} as cnt FROM {table} WHERE tenant_id = {} GROUP BY status",
                    crate::db::Driver::ph(1)
                );
                let rows: Vec<(String, i64)> =
                    sqlx::query_as::<crate::db::pool::Db, (String, i64)>(&sql)
                        .bind(&tid)
                        .fetch_all(&self.pool)
                        .await
                        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;
                rows
            } else {
                let sql = format!("SELECT status, {cnt_expr} as cnt FROM {table} GROUP BY status");
                let rows: Vec<(String, i64)> =
                    sqlx::query_as::<crate::db::pool::Db, (String, i64)>(&sql)
                        .fetch_all(&self.pool)
                        .await
                        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;
                rows
            };

            let mut by_status = serde_json::Map::new();
            for (status, count) in status_sql {
                by_status.insert(status, json!(count));
            }
            if let Some(obj) = result.as_object_mut() {
                obj.insert("by_status".into(), json!(by_status));
            }
        }

        Ok(result)
    }

    /// Trend data (daily creation counts over the last N days)
    pub async fn trends(
        &self,
        table: &str,
        days: i64,
        tenant_id: Option<&str>,
    ) -> Result<Value, AppError> {
        validate_table_name(table)?;
        let days = days.clamp(1, 365);
        let has_ts = has_column(&self.pool, table, "created_at").await;
        let has_tenant = crate::db::tenant::has_tenant_id(&self.pool, table).await;

        if !has_ts {
            return Ok(json!({
                "table": table,
                "days": days,
                "data": [],
            }));
        }

        let date_expr = date_trunc_day_expr("created_at");

        let cnt_expr = crate::db::Driver::cast_int("COUNT(*)");
        let ago = crate::db::Driver::ago_expr(days);
        let sql = if has_tenant {
            format!(
                "SELECT {date_expr} as d, {cnt_expr} as cnt FROM {table} \
                 WHERE tenant_id = {} AND created_at >= {ago} \
                 GROUP BY d ORDER BY d",
                crate::db::Driver::ph(1)
            )
        } else {
            format!(
                "SELECT {date_expr} as d, {cnt_expr} as cnt FROM {table} \
                 WHERE created_at >= {ago} \
                 GROUP BY d ORDER BY d"
            )
        };

        let mut q = sqlx::query_as::<crate::db::pool::Db, (String, i64)>(&sql);
        if has_tenant {
            let tid = crate::db::tenant::resolve_tenant(tenant_id).to_string();
            q = q.bind(tid);
        }

        let rows: Vec<(String, i64)> = q
            .fetch_all(&self.pool)
            .await
            .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;

        let data: Vec<Value> = rows
            .into_iter()
            .map(|(date, count)| json!({"date": date, "count": count}))
            .collect();

        Ok(json!({
            "table": table,
            "days": days,
            "data": data,
        }))
    }

    /// Count records per content type
    async fn count_content_types(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<serde_json::Map<String, Value>, AppError> {
        let tables = get_content_tables(&self.pool).await?;
        let mut result = serde_json::Map::new();

        for table in &tables {
            let tf = crate::db::tenant::tenant_filter_ph(tenant_id, 1);
            let count = count_table(&self.pool, table, &tf, tenant_id).await?;
            result.insert(table.clone(), json!(count));
        }

        Ok(result)
    }

    /// Sum total revenue from completed orders
    async fn sum_revenue(&self, tenant_id: Option<&str>) -> Result<i64, AppError> {
        let tf = crate::db::tenant::tenant_filter_ph(tenant_id, 1);
        let sum_expr = crate::db::Driver::cast_int("COALESCE(SUM(total_amount), 0)");
        let sql = format!("SELECT {sum_expr} FROM orders WHERE status = 'completed'{tf}");
        let mut q = sqlx::query_scalar::<crate::db::pool::Db, i64>(&sql);
        if let Some(tid) = tenant_id {
            q = q.bind(crate::db::tenant::resolve_tenant(Some(tid)));
        }
        let result: i64 = q
            .fetch_one(&self.pool)
            .await
            .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;
        Ok(result)
    }

    /// Count records grouped by status
    async fn count_by_status(
        &self,
        table: &str,
        tenant_id: Option<&str>,
    ) -> Result<serde_json::Map<String, Value>, AppError> {
        validate_table_name(table)?;
        let has_status = has_column(&self.pool, table, "status").await;
        if !has_status {
            return Ok(serde_json::Map::new());
        }

        let has_tenant = crate::db::tenant::has_tenant_id(&self.pool, table).await;

        let cnt_expr = crate::db::Driver::cast_int("COUNT(*)");
        let rows: Vec<(String, i64)> = if has_tenant {
            let tid = crate::db::tenant::resolve_tenant(tenant_id).to_string();
            let sql = format!(
                "SELECT status, {cnt_expr} as cnt FROM {table} WHERE tenant_id = {} GROUP BY status",
                crate::db::Driver::ph(1)
            );
            sqlx::query_as::<crate::db::pool::Db, (String, i64)>(&sql)
                .bind(&tid)
                .fetch_all(&self.pool)
                .await
                .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?
        } else {
            let sql = format!("SELECT status, {cnt_expr} as cnt FROM {table} GROUP BY status");
            sqlx::query_as::<crate::db::pool::Db, (String, i64)>(&sql)
                .fetch_all(&self.pool)
                .await
                .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?
        };

        let mut map = serde_json::Map::new();
        for (status, count) in rows {
            map.insert(status, json!(count));
        }
        Ok(map)
    }

    /// Recent activity (most recently created posts, orders, products + comments)
    async fn recent_activity(
        &self,
        tenant_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Value>, AppError> {
        raisfast_derive::check_schema!("posts", "id", "title", "slug", "created_at");
        raisfast_derive::check_schema!("comments", "content", "created_at");
        raisfast_derive::check_schema!("orders", "id", "order_no", "total_amount", "created_at");
        raisfast_derive::check_schema!("products", "id", "title", "created_at");
        let mut activities = Vec::new();

        let tf_aliased = crate::db::tenant::tenant_filter_aliased_ph("p", tenant_id, 1);
        let limit_clause = format!("LIMIT {limit}");

        let post_sql = format!(
            "SELECT p.id, p.title, p.slug, {} FROM posts p WHERE 1=1{tf_aliased} \
             ORDER BY p.created_at DESC {limit_clause}",
            crate::db::Driver::cast_ts("p.created_at")
        );

        let posts: Vec<(i64, Option<String>, String, String)> = raisfast_derive::crud_query!(
            &self.pool,
            (i64, Option<String>, String, String),
            &post_sql,
            [],
            fetch_all,
            tenant: tenant_id
        )
        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;

        for (raw_id, title, slug, at) in posts {
            let encoded_id = crate::types::snowflake_id::encode_id(raw_id);
            activities.push(json!({
                "type": "post.created",
                "id": encoded_id,
                "title": title.unwrap_or_default(),
                "slug": slug,
                "at": at,
            }));
        }

        let comment_sql = format!(
            "SELECT c.content, {} FROM comments c WHERE 1=1{tf_aliased} \
             ORDER BY c.created_at DESC {limit_clause}",
            crate::db::Driver::cast_ts("c.created_at")
        );

        let comments: Vec<(Option<String>, String)> = raisfast_derive::crud_query!(
            &self.pool,
            (Option<String>, String),
            &comment_sql,
            [],
            fetch_all,
            tenant: tenant_id
        )
        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;

        for (content, at) in comments {
            activities.push(json!({
                "type": "comment.created",
                "content": content.unwrap_or_default(),
                "at": at,
            }));
        }

        let tf = crate::db::tenant::tenant_filter_ph(tenant_id, 1);
        let order_sql = format!(
            "SELECT id, order_no, total_amount, {} FROM orders WHERE 1=1{tf} \
             ORDER BY created_at DESC {limit_clause}",
            crate::db::Driver::cast_ts("created_at")
        );
        let orders: Vec<(i64, String, i64, String)> = raisfast_derive::crud_query!(
            &self.pool,
            (i64, String, i64, String),
            &order_sql,
            [],
            fetch_all,
            tenant: tenant_id
        )
        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;

        for (raw_id, order_no, total_amount, at) in orders {
            let encoded_id = crate::types::snowflake_id::encode_id(raw_id);
            activities.push(json!({
                "type": "order.created",
                "id": encoded_id,
                "title": order_no,
                "amount": total_amount,
                "at": at,
            }));
        }

        let product_sql = format!(
            "SELECT id, title, {} FROM products WHERE 1=1{tf} \
             ORDER BY created_at DESC {limit_clause}",
            crate::db::Driver::cast_ts("created_at")
        );
        let products: Vec<(i64, Option<String>, String)> = raisfast_derive::crud_query!(
            &self.pool,
            (i64, Option<String>, String),
            &product_sql,
            [],
            fetch_all,
            tenant: tenant_id
        )
        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;

        for (raw_id, title, at) in products {
            let encoded_id = crate::types::snowflake_id::encode_id(raw_id);
            activities.push(json!({
                "type": "product.created",
                "id": encoded_id,
                "title": title.unwrap_or_default(),
                "at": at,
            }));
        }

        activities.sort_by(|a, b| {
            let at_a = a["at"].as_str().unwrap_or("");
            let at_b = b["at"].as_str().unwrap_or("");
            at_b.cmp(at_a)
        });
        activities.truncate(limit as usize);

        Ok(activities)
    }
}

/// Count records in a table
async fn count_table(
    pool: &Pool,
    table: &str,
    tenant_filter: &str,
    tenant_id: Option<&str>,
) -> Result<i64, AppError> {
    validate_table_name(table)?;
    let cnt_expr = crate::db::Driver::cast_int("COUNT(*)");
    let sql = format!("SELECT {cnt_expr} FROM {table} WHERE 1=1{tenant_filter}");
    let mut q = sqlx::query_scalar::<crate::db::pool::Db, i64>(&sql);
    if tenant_id.is_some() {
        q = q.bind(crate::db::tenant::resolve_tenant(tenant_id));
    }
    let result: i64 = q
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;
    Ok(result)
}

/// Check if a table has a specific column
async fn has_column(pool: &Pool, table: &str, column: &str) -> bool {
    crate::db::Driver::has_column(pool, table, column).await
}

/// Get all content-type-related table names from the database
async fn get_content_tables(pool: &Pool) -> Result<Vec<String>, AppError> {
    let excluded_tables = "'users','refresh_tokens','media','plugin_storage','roles','permissions','options','tenants','pending_jobs','cron_schedules','cron_execution_log'";
    Ok(crate::db::Driver::list_user_tables(pool, excluded_tables).await)
}

/// Date truncation expression (truncate to day)
fn date_trunc_day_expr(col: &str) -> String {
    crate::db::Driver::date_trunc_day(col)
}

fn validate_table_name(table: &str) -> Result<(), AppError> {
    if crate::db::driver::is_safe_identifier(table) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("invalid table name: {table}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stats_overview_empty_db() {
        let pool = Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, slug TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default', username TEXT NOT NULL, role TEXT NOT NULL, status TEXT NOT NULL, registered_via TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE comments (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE media (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("CREATE TABLE categories (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE tags (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE products (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, order_no TEXT NOT NULL, total_amount INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE coupons (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let svc = StatsService::new(pool);
        let result = svc.overview(None).await.unwrap();

        assert_eq!(result["total_posts"], 0);
        assert_eq!(result["total_users"], 0);
        assert_eq!(result["total_comments"], 0);
        assert_eq!(result["total_media"], 0);
        assert_eq!(result["total_products"], 0);
        assert_eq!(result["total_orders"], 0);
        assert_eq!(result["total_coupons"], 0);
        assert_eq!(result["total_revenue"], 0);
    }

    #[tokio::test]
    async fn stats_overview_with_data() {
        let pool = Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, slug TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default', username TEXT NOT NULL, role TEXT NOT NULL, status TEXT NOT NULL, registered_via TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE comments (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE media (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("CREATE TABLE categories (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE tags (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE products (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, order_no TEXT NOT NULL, total_amount INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE coupons (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO posts (id, title, slug, created_at) VALUES (1, 'Hello', 'hello', '2024-01-01T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, username, role, status, registered_via) VALUES (1, 'user1', 'reader', 'active', 'email')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO orders (id, order_no, total_amount, status, created_at) VALUES (1, 'ORD-001', 9900, 'completed', '2024-01-02T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        let svc = StatsService::new(pool);
        let result = svc.overview(None).await.unwrap();

        assert_eq!(result["total_posts"], 1);
        assert_eq!(result["total_users"], 1);
        assert_eq!(result["total_comments"], 0);
        assert_eq!(result["total_orders"], 1);
        assert_eq!(result["total_revenue"], 9900);

        let activity = result["recent_activity"].as_array().unwrap();
        assert!(!activity.is_empty());
    }

    #[tokio::test]
    async fn stats_content_stats_with_status() {
        let pool = Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE ct_test (id INTEGER PRIMARY KEY AUTOINCREMENT, status TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO ct_test (id, status) VALUES (1, 'draft')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ct_test (id, status) VALUES (2, 'published')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ct_test (id, status) VALUES (3, 'published')")
            .execute(&pool)
            .await
            .unwrap();

        let svc = StatsService::new(pool);
        let result = svc.content_stats("ct_test", None).await.unwrap();

        assert_eq!(result["total"], 3);
        assert_eq!(result["by_status"]["draft"], 1);
        assert_eq!(result["by_status"]["published"], 2);
    }

    #[tokio::test]
    async fn stats_trends() {
        let pool = Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE ct_trends (id INTEGER PRIMARY KEY AUTOINCREMENT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        sqlx::query("INSERT INTO ct_trends (id, created_at) VALUES (1, ?)")
            .bind(&today)
            .execute(&pool)
            .await
            .unwrap();

        let svc = StatsService::new(pool);
        let result = svc.trends("ct_trends", 7, None).await.unwrap();

        assert_eq!(result["days"], 7);
        let data = result["data"].as_array().unwrap();
        assert!(!data.is_empty());
        assert_eq!(data[0]["count"], 1);
    }
}

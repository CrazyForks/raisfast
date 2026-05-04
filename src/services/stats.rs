//! 仪表盘统计服务
//!
//! 提供 Admin Dashboard 所需的聚合统计数据：
//! - 总览统计（各实体总数、内容类型分布、近期活动）
//! - 单个内容类型统计（状态分布）
//! - 趋势数据（近 N 天的创建量）

use serde_json::{Value, json};

use crate::db::Pool;
use crate::errors::app_error::AppError;

/// 仪表盘统计服务
pub struct StatsService {
    pool: Pool,
}

impl StatsService {
    /// 创建统计服务实例
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// 总览统计
    ///
    /// 返回各实体总数、content type 分布、近期活动
    pub async fn overview(&self, tenant_id: Option<&str>) -> Result<Value, AppError> {
        let tf = crate::db::tenant::tenant_filter(tenant_id);
        let tf_aliased = crate::db::tenant::tenant_filter_aliased("p", tenant_id);

        let total_posts = count_table(&self.pool, "posts", tf_aliased.as_str(), tenant_id).await?;
        let total_comments =
            count_table(&self.pool, "comments", tf_aliased.as_str(), tenant_id).await?;
        let total_users = count_table(&self.pool, "users", tf, tenant_id).await?;
        let total_media = count_table(&self.pool, "media", tf, tenant_id).await?;
        let total_categories =
            count_table(&self.pool, "categories", tf_aliased.as_str(), tenant_id).await?;
        let total_tags = count_table(&self.pool, "tags", tf_aliased.as_str(), tenant_id).await?;

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
            "posts_by_status": posts_by_status,
            "comments_by_status": comments_by_status,
            "content_by_type": content_by_type,
            "recent_activity": recent_activity,
        }))
    }

    /// 单个内容类型统计（状态分布）
    pub async fn content_stats(
        &self,
        table: &str,
        tenant_id: Option<&str>,
    ) -> Result<Value, AppError> {
        let tf = crate::db::tenant::tenant_filter(tenant_id);

        let has_status = has_column(&self.pool, table, "status").await;
        let has_tenant = crate::db::tenant::has_tenant_id(&self.pool, table).await;

        let total = count_table(&self.pool, table, tf, tenant_id).await?;

        let mut result = json!({
            "table": table,
            "total": total,
        });

        if has_status {
            let status_sql = if has_tenant {
                let tid = crate::db::tenant::resolve_tenant(tenant_id).to_string();
                let sql = format!(
                    "SELECT status, COUNT(*) as cnt FROM {table} WHERE tenant_id = ? GROUP BY status"
                );
                let sql = crate::db::dialect::translate(&sql);
                let rows: Vec<(String, i64)> = sqlx::query_as::<_, (String, i64)>(&sql)
                    .bind(&tid)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
                rows
            } else {
                let sql = format!("SELECT status, COUNT(*) as cnt FROM {table} GROUP BY status");
                let sql = crate::db::dialect::translate(&sql);
                let rows: Vec<(String, i64)> = sqlx::query_as::<_, (String, i64)>(&sql)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
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

    /// 趋势数据（近 N 天每天创建量）
    pub async fn trends(
        &self,
        table: &str,
        days: i64,
        tenant_id: Option<&str>,
    ) -> Result<Value, AppError> {
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

        let sql = if has_tenant {
            format!(
                "SELECT {date_expr} as d, COUNT(*) as cnt FROM {table} \
                 WHERE tenant_id = ? AND created_at >= datetime('now', '-{days} days') \
                 GROUP BY d ORDER BY d"
            )
        } else {
            format!(
                "SELECT {date_expr} as d, COUNT(*) as cnt FROM {table} \
                 WHERE created_at >= datetime('now', '-{days} days') \
                 GROUP BY d ORDER BY d"
            )
        };
        let sql = crate::db::dialect::translate(&sql);

        let mut q = sqlx::query_as::<_, (String, i64)>(&sql);
        if has_tenant {
            let tid = crate::db::tenant::resolve_tenant(tenant_id).to_string();
            q = q.bind(tid);
        }

        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

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

    /// 统计各 content type 的记录数
    async fn count_content_types(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<serde_json::Map<String, Value>, AppError> {
        let tables = get_content_tables(&self.pool).await?;
        let mut result = serde_json::Map::new();

        for table in &tables {
            let tf = crate::db::tenant::tenant_filter(tenant_id);
            let count = count_table(&self.pool, table, tf, tenant_id).await?;
            result.insert(table.clone(), json!(count));
        }

        Ok(result)
    }

    /// 按状态分组计数
    async fn count_by_status(
        &self,
        table: &str,
        tenant_id: Option<&str>,
    ) -> Result<serde_json::Map<String, Value>, AppError> {
        let has_status = has_column(&self.pool, table, "status").await;
        if !has_status {
            return Ok(serde_json::Map::new());
        }

        let has_tenant = crate::db::tenant::has_tenant_id(&self.pool, table).await;

        let rows = if has_tenant {
            let tid = crate::db::tenant::resolve_tenant(tenant_id).to_string();
            let sql = format!(
                "SELECT status, COUNT(*) as cnt FROM {table} WHERE tenant_id = ? GROUP BY status"
            );
            let sql = crate::db::dialect::translate(&sql);
            sqlx::query_as::<_, (String, i64)>(&sql)
                .bind(&tid)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
        } else {
            let sql = format!("SELECT status, COUNT(*) as cnt FROM {table} GROUP BY status");
            let sql = crate::db::dialect::translate(&sql);
            sqlx::query_as::<_, (String, i64)>(&sql)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
        };

        let mut map = serde_json::Map::new();
        for (status, count) in rows {
            map.insert(status, json!(count));
        }
        Ok(map)
    }

    /// 近期活动（最近创建的 posts + comments）
    async fn recent_activity(
        &self,
        tenant_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Value>, AppError> {
        let mut activities = Vec::new();

        let tf_aliased = crate::db::tenant::tenant_filter_aliased("p", tenant_id);
        let limit_clause = format!("LIMIT {limit}");

        let post_sql = format!(
            "SELECT p.title, p.slug, p.created_at FROM posts p WHERE 1=1{tf_aliased} \
             ORDER BY p.created_at DESC {limit_clause}"
        );
        let post_sql = crate::db::dialect::translate(&post_sql);

        let mut post_q = sqlx::query_as::<_, (Option<String>, String, String)>(&post_sql);
        if let Some(tid) = tenant_id {
            post_q = post_q.bind(tid);
        }
        let posts = post_q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        for (title, slug, at) in posts {
            activities.push(json!({
                "type": "post.created",
                "title": title.unwrap_or_default(),
                "slug": slug,
                "at": at,
            }));
        }

        let comment_sql = format!(
            "SELECT c.content, c.created_at FROM comments c WHERE 1=1{tf_aliased} \
             ORDER BY c.created_at DESC {limit_clause}"
        );
        let comment_sql = crate::db::dialect::translate(&comment_sql);

        let mut comment_q = sqlx::query_as::<_, (Option<String>, String)>(&comment_sql);
        if let Some(tid) = tenant_id {
            comment_q = comment_q.bind(tid);
        }
        let comments = comment_q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        for (content, at) in comments {
            activities.push(json!({
                "type": "comment.created",
                "content": content.unwrap_or_default(),
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

/// COUNT 某张表的记录数
async fn count_table(
    pool: &Pool,
    table: &str,
    tenant_filter: &str,
    tenant_id: Option<&str>,
) -> Result<i64, AppError> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE 1=1{tenant_filter}");
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query_scalar::<_, i64>(&sql);
    if tenant_id.is_some() {
        q = q.bind(crate::db::tenant::resolve_tenant(tenant_id));
    }
    q.fetch_one(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
}

/// 检查表是否有某列
async fn has_column(pool: &Pool, table: &str, column: &str) -> bool {
    #[cfg(feature = "db-sqlite")]
    {
        let sql = format!("PRAGMA table_info({table})");
        let rows = sqlx::query_as::<_, (i32, String, String, bool, Option<String>, bool)>(&sql)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        rows.iter().any(|(_, name, _, _, _, _)| name == column)
    }
    #[cfg(not(feature = "db-sqlite"))]
    {
        let _ = (pool, table, column);
        false
    }
}

/// 获取数据库中所有 content type 相关的表名
async fn get_content_tables(pool: &Pool) -> Result<Vec<String>, AppError> {
    #[cfg(feature = "db-sqlite")]
    {
        let sql = "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('users','refresh_tokens','media','plugin_storage','roles','permissions','options','tenants','pending_jobs','cron_schedules','cron_execution_log')";
        let rows = sqlx::query_as::<_, (String,)>(sql)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }
    #[cfg(not(feature = "db-sqlite"))]
    {
        let _ = pool;
        Ok(vec![])
    }
}

/// SQLite 日期截断表达式（截断到天）
fn date_trunc_day_expr(col: &str) -> String {
    format!("DATE({col})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stats_overview_empty_db() {
        let pool = Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE posts (id TEXT PRIMARY KEY, title TEXT, slug TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE users (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE comments (id TEXT PRIMARY KEY, content TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE media (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("CREATE TABLE categories (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE tags (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')",
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
    }

    #[tokio::test]
    async fn stats_overview_with_data() {
        let pool = Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE posts (id TEXT PRIMARY KEY, title TEXT, slug TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE users (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE comments (id TEXT PRIMARY KEY, content TEXT, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE media (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("CREATE TABLE categories (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE tags (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO posts (id, title, slug, created_at) VALUES ('p1', 'Hello', 'hello', '2024-01-01T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id) VALUES ('u1')")
            .execute(&pool)
            .await
            .unwrap();

        let svc = StatsService::new(pool);
        let result = svc.overview(None).await.unwrap();

        assert_eq!(result["total_posts"], 1);
        assert_eq!(result["total_users"], 1);
        assert_eq!(result["total_comments"], 0);

        let activity = result["recent_activity"].as_array().unwrap();
        assert!(!activity.is_empty());
        assert_eq!(activity[0]["type"], "post.created");
    }

    #[tokio::test]
    async fn stats_content_stats_with_status() {
        let pool = Pool::connect(":memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE ct_test (id TEXT PRIMARY KEY, status TEXT, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO ct_test (id, status) VALUES ('1', 'draft')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ct_test (id, status) VALUES ('2', 'published')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ct_test (id, status) VALUES ('3', 'published')")
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
            "CREATE TABLE ct_trends (id TEXT PRIMARY KEY, created_at TEXT NOT NULL, tenant_id TEXT NOT NULL DEFAULT 'default')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        sqlx::query("INSERT INTO ct_trends (id, created_at) VALUES ('1', ?)")
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

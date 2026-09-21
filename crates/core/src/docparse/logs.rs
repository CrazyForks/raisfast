//! docparse_job_logs — 文档转换/图像识别用量账本（§7 商业化数据前置）。
//!
//! 一张表覆盖两类 job（`kind` 区分 convert/recognize）；运行时查询
//! （sqlx::query + Driver 占位符，跨库），不走 CRUD 宏——账本表无强类型
//! 装配需求，且避免新表触发 sqlx 离线缓存重建。

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::db::{DbDriver as _, Driver};
use crate::errors::app_error::{AppError, AppResult};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ParseJobLog {
    pub id: i64,
    pub tenant_id: String,
    pub kind: String,
    pub status: String,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub filename: Option<String>,
    pub input_bytes: Option<i64>,
    pub pages: Option<i64>,
    pub chars: Option<i64>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub result_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// job 提交即落账（status=queued）——计费字段随后续 update 补齐。
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &crate::db::Pool,
    id: i64,
    tenant: &str,
    kind: &str,
    status: &str,
    engine: Option<&str>,
    model: Option<&str>,
    filename: &str,
    input_bytes: i64,
    result_key: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO docparse_job_logs \
         (id, tenant_id, kind, status, engine, model, filename, input_bytes, \
          result_key, created_at) \
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
        Driver::ph(8),
        Driver::ph(9),
        Driver::ph(10)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(id)
        .bind(tenant)
        .bind(kind)
        .bind(status)
        .bind(engine)
        .bind(model)
        .bind(filename)
        .bind(input_bytes)
        .bind(result_key)
        .bind(crate::utils::tz::now_utc())
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("docparse_job_logs insert: {e}")))?;
    Ok(())
}

/// job 终态回填（completed/failed）——计费字段在此定稿。
#[allow(clippy::too_many_arguments)]
pub async fn finish(
    pool: &crate::db::Pool,
    id: i64,
    status: &str,
    engine: Option<&str>,
    pages: Option<i64>,
    chars: Option<i64>,
    duration_ms: Option<i64>,
    error: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE docparse_job_logs SET status = {}, engine = {}, pages = {}, chars = {}, \
         duration_ms = {}, error = {}, finished_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
        Driver::ph(8)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(status)
        .bind(engine)
        .bind(pages)
        .bind(chars)
        .bind(duration_ms)
        .bind(error)
        .bind(crate::utils::tz::now_utc())
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("docparse_job_logs finish: {e}")))?;
    Ok(())
}

/// 按租户列出最近用量（ops 视图 / M4 报表数据源）。
pub async fn list_by_tenant(
    pool: &crate::db::Pool,
    tenant: &str,
    limit: i64,
) -> AppResult<Vec<ParseJobLog>> {
    let sql = format!(
        "SELECT * FROM docparse_job_logs WHERE tenant_id = {} \
         ORDER BY created_at DESC LIMIT {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    let rows: Vec<ParseJobLog> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(tenant)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("docparse_job_logs list: {e}")))?;
    Ok(rows)
}

/// 按状态清理（复用 retention sweep 模式；M4 归档前置）。
pub async fn sweep_before(pool: &crate::db::Pool, cutoff: DateTime<Utc>) -> AppResult<u64> {
    let sql = format!(
        "DELETE FROM docparse_job_logs WHERE created_at < {}",
        Driver::ph(1)
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(cutoff)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("docparse_job_logs sweep: {e}")))?;
    Ok(result.rows_affected())
}

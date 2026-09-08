//! `kb_query_logs` table — every ask is logged (kb-technical-design §9).
//!
//! `status='uncovered'` or negative feedback rows feed the admin gap list
//! (M5 sedimentation loop).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbQueryLog {
    pub id: SnowflakeId,
    pub kb_id: Option<SnowflakeId>,
    pub question: String,
    pub answer: Option<String>,
    pub cited_units: Option<Value>,
    /// `answered` | `uncovered` | `error`.
    pub status: String,
    pub top_score: Option<f64>,
    pub feedback: Option<i64>,
    pub user_id: Option<SnowflakeId>,
    pub created_at: Timestamp,
}

/// Insert a query log row and return its id.
#[allow(clippy::too_many_arguments)]
pub async fn insert_log(
    pool: &crate::db::Pool,
    kb_id: Option<SnowflakeId>,
    question: &str,
    answer: Option<&str>,
    cited_units: Option<&Value>,
    status: &str,
    top_score: Option<f32>,
    user_id: Option<SnowflakeId>,
) -> AppResult<SnowflakeId> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_query_logs",
        [
            "id" => id,
            "kb_id" => kb_id,
            "question" => question,
            "answer" => answer,
            "cited_units" => cited_units,
            "status" => status,
            "top_score" => top_score,
            "user_id" => user_id,
            "created_at" => now
        ]
    )?;
    Ok(id)
}

/// Knowledge-gap list: uncovered (or negatively rated) questions grouped
/// with occurrence counts — the M5 sedimentation input (§9).
pub async fn list_gaps(pool: &crate::db::Pool, limit: i64) -> AppResult<Vec<(String, i64)>> {
    let placeholders = (1..=1).map(Driver::ph).collect::<Vec<_>>().join("");
    let sql = format!(
        "SELECT question, {} FROM kb_query_logs WHERE status = 'uncovered' OR feedback < 0 \
         GROUP BY question ORDER BY 2 DESC",
        Driver::cast_int("COUNT(*)"),
    );
    let _ = placeholders;
    let sql = format!("{sql} LIMIT {}", Driver::ph(1));
    let rows: Vec<(String, i64)> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
        })?;
    Ok(rows)
}

pub async fn find_log_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<Option<KbQueryLog>> {
    Ok(raisfast_derive::crud_find!(pool, "kb_query_logs", KbQueryLog, where: ("id", id))?)
}

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
    /// S1-rewritten question (observability §3.4; the retrieval/generation
    /// text — `question` keeps the user's original).
    pub rewritten_question: Option<String>,
    /// Full KB scope of the ask (the `kb_id` column only keeps the first).
    pub kb_ids: Option<Value>,
    pub latency_ms: Option<i64>,
    /// Backlink to the traced run (`kb_runs.id`).
    pub run_id: Option<SnowflakeId>,
    /// Error text when `status='error'`.
    pub error: Option<String>,
    /// 'ask' | 'search' | 'agent' — gaps aggregates 'ask' by default.
    pub source: String,
    pub created_at: Timestamp,
}

/// Rich insert entry for the observability plane (§3.4) — every ask entry
/// point (`/kb/ask`, `/kb/search`, agent `knowledge_search`) logs through
/// this so the three planes become indistinguishable in `kb_query_logs`.
pub struct LogEntry<'a> {
    pub kb_id: Option<SnowflakeId>,
    pub question: &'a str,
    pub answer: Option<&'a str>,
    pub cited_units: Option<&'a Value>,
    pub status: &'a str,
    pub top_score: Option<f64>,
    pub user_id: Option<SnowflakeId>,
    pub rewritten_question: Option<&'a str>,
    pub kb_ids: Option<&'a Value>,
    pub latency_ms: Option<i64>,
    pub run_id: Option<SnowflakeId>,
    pub error: Option<&'a str>,
    pub source: &'a str,
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
    top_score: Option<f64>,
    user_id: Option<SnowflakeId>,
) -> AppResult<SnowflakeId> {
    insert_entry(
        pool,
        &LogEntry {
            kb_id,
            question,
            answer,
            cited_units,
            status,
            top_score,
            user_id,
            rewritten_question: None,
            kb_ids: None,
            latency_ms: None,
            run_id: None,
            error: None,
            source: "ask",
        },
    )
    .await
}

/// Insert a full observability log row (§3.4).
pub async fn insert_entry(pool: &crate::db::Pool, entry: &LogEntry<'_>) -> AppResult<SnowflakeId> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_query_logs",
        [
            "id" => id,
            "kb_id" => entry.kb_id,
            "question" => entry.question,
            "answer" => entry.answer,
            "cited_units" => entry.cited_units,
            "status" => entry.status,
            "top_score" => entry.top_score,
            "user_id" => entry.user_id,
            "rewritten_question" => entry.rewritten_question,
            "kb_ids" => entry.kb_ids,
            "latency_ms" => entry.latency_ms,
            "run_id" => entry.run_id,
            "error" => entry.error,
            "source" => entry.source,
            "created_at" => now
        ]
    )?;
    Ok(id)
}

/// Knowledge-gap list: uncovered (or negatively rated) questions grouped
/// with occurrence counts — the M5 sedimentation input (§9). Defaults to
/// `source='ask'` (user questions); agent/search probes are gap *signal*
/// too but stay out of the human curation list (§3.4 论证校准).
pub async fn list_gaps(pool: &crate::db::Pool, limit: i64) -> AppResult<Vec<(String, i64, i64)>> {
    let sql = format!(
        "SELECT question, {}, {} FROM kb_query_logs \
         WHERE (status = 'uncovered' OR feedback < 0) AND source = 'ask' \
         GROUP BY question ORDER BY 2 DESC LIMIT {}",
        Driver::cast_int("COUNT(*)"),
        Driver::cast_int("MAX(id)"),
        Driver::ph(1)
    );
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(crate::db::safe_sql(&sql))
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

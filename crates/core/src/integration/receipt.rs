//! `itg_receipts` model — idempotency + full inbound trace (integration.md §3.2).
//!
//! All write helpers run on a caller-owned transaction (`&mut DbConnection`);
//! the pipeline composes them atomically per integration.md §6.3.

use serde_json::Value;

use crate::db::driver::DbDriver;
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

pub const STATUS_RECEIVED: &str = "received";
pub const STATUS_RETRYING: &str = "retrying";
pub const STATUS_DELIVERED: &str = "delivered";
pub const STATUS_DEAD: &str = "dead";
pub const STATUS_DUPLICATE: &str = "duplicate";

/// SHA-256 hex of the raw body — the duplicate-detection fingerprint.
#[must_use]
pub fn payload_hash(body: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(body))
}

/// Insert-ignore a receipt. Returns `Some(id)` when this call created the row
/// (first delivery), `None` when `(channel_id, external_id)` already exists.
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn insert_ignore_tx(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    channel_id: SnowflakeId,
    external_id: &str,
    kind: &str,
    hash: &str,
    received_at: Timestamp,
) -> AppResult<Option<SnowflakeId>> {
    let sql = crate::db::Driver::insert_ignore_sql(
        "itg_receipts",
        "id, channel_id, external_id, kind, payload_hash, status, attempts, received_at",
        &format!(
            "{}, {}, {}, {}, {}, {}, {}, {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3),
            crate::db::Driver::ph(4),
            crate::db::Driver::ph(5),
            crate::db::Driver::ph(6),
            crate::db::Driver::ph(7),
            crate::db::Driver::ph(8)
        ),
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(*id)
        .bind(*channel_id)
        .bind(external_id)
        .bind(kind)
        .bind(hash)
        .bind(STATUS_RECEIVED)
        .bind(0_i64)
        .bind(received_at)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() > 0 {
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

/// First-pass completion: envelope snapshot, step timeline (with pending
/// placeholders for planned jobs), target id, delivered timestamp.
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn mark_delivered_tx(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    envelope: &Value,
    steps: &Value,
    target_id: Option<SnowflakeId>,
    delivered_at: Timestamp,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE itg_receipts SET status = {}, envelope = {}, steps = {}, target_id = {}, \
         delivered_at = {} WHERE id = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        crate::db::Driver::ph(3),
        crate::db::Driver::ph(4),
        crate::db::Driver::ph(5),
        crate::db::Driver::ph(6)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(STATUS_DELIVERED)
        .bind(envelope)
        .bind(steps)
        .bind(target_id)
        .bind(delivered_at)
        .bind(*id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Full receipt row for the retry/replay runners.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ReceiptRow {
    pub id: SnowflakeId,
    pub channel_id: SnowflakeId,
    pub external_id: String,
    pub kind: String,
    pub status: String,
    pub attempts: i64,
    pub next_retry_at: Option<Timestamp>,
    pub envelope: Option<Value>,
    pub steps: Option<Value>,
    pub target_id: Option<SnowflakeId>,
}

/// Load a receipt by id (pool).
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<Option<ReceiptRow>> {
    let sql = format!(
        "SELECT id, channel_id, external_id, kind, status, attempts, next_retry_at, envelope,          steps, target_id FROM itg_receipts WHERE id = {}",
        crate::db::Driver::ph(1)
    );
    let row: Option<ReceiptRow> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(*id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Existing-row fingerprint + state for the duplicate/external-redelivery branch.
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn find_state_tx(
    tx: &mut crate::db::pool::DbConnection,
    channel_id: SnowflakeId,
    external_id: &str,
) -> AppResult<Option<ReceiptState>> {
    let sql = format!(
        "SELECT id, payload_hash, status, attempts, envelope FROM itg_receipts \
         WHERE channel_id = {} AND external_id = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2)
    );
    let row: Option<ReceiptState> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(*channel_id)
        .bind(external_id)
        .fetch_optional(&mut *tx)
        .await?;
    Ok(row)
}

/// Duplicate-branch lookup payload.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReceiptState {
    pub id: SnowflakeId,
    pub payload_hash: String,
    pub status: String,
    pub attempts: i64,
    pub envelope: Option<Value>,
}

/// Transition to `retrying`: bump attempts, schedule next run, persist snapshot.
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn mark_retrying_tx(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    attempts: i64,
    next_retry_at: Timestamp,
    envelope: &Value,
    steps: &Value,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE itg_receipts SET status = {}, attempts = {}, next_retry_at = {}, envelope = {}, \
         steps = {} WHERE id = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        crate::db::Driver::ph(3),
        crate::db::Driver::ph(4),
        crate::db::Driver::ph(5),
        crate::db::Driver::ph(6)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(STATUS_RETRYING)
        .bind(attempts)
        .bind(next_retry_at)
        .bind(envelope)
        .bind(steps)
        .bind(*id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Terminal failure: `dead`.
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn mark_dead_tx(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    steps: &Value,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE itg_receipts SET status = {}, steps = {} WHERE id = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        crate::db::Driver::ph(3)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(STATUS_DEAD)
        .bind(steps)
        .bind(*id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Cross-transaction step append (§10.7): read-modify-write the steps JSON in
/// its own small transaction. Idempotency is the caller's responsibility
/// (step name + attempt convention).
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn append_step(pool: &crate::db::Pool, id: SnowflakeId, entry: &Value) -> AppResult<()> {
    crate::in_transaction!(pool, tx, {
        let sql = format!(
            "SELECT steps FROM itg_receipts WHERE id = {}",
            crate::db::Driver::ph(1)
        );
        let existing: Option<(Option<Value>,)> = sqlx::query_as(crate::db::safe_sql(&sql))
            .bind(*id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some((steps,)) = existing else {
            return Ok::<(), crate::errors::app_error::AppError>(());
        };
        let mut arr = steps
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        arr.push(entry.clone());
        let updated = Value::Array(arr);
        let usql = format!(
            "UPDATE itg_receipts SET steps = {} WHERE id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2)
        );
        sqlx::query(crate::db::safe_sql(&usql))
            .bind(&updated)
            .bind(*id)
            .execute(&mut *tx)
            .await?;
        Ok(())
    })
}

/// Flip a `pending` step placeholder to a terminal state (job completion
/// writeback, §10.7). Idempotent: already-terminal entries are left as-is.
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn flip_pending_step(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    job_type: &str,
    ok: bool,
    detail: &str,
) -> AppResult<()> {
    crate::in_transaction!(pool, tx, {
        let sql = format!(
            "SELECT steps FROM itg_receipts WHERE id = {}",
            crate::db::Driver::ph(1)
        );
        let existing: Option<(Option<Value>,)> = sqlx::query_as(crate::db::safe_sql(&sql))
            .bind(*id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some((steps,)) = existing else {
            return Ok::<(), crate::errors::app_error::AppError>(());
        };
        let Some(mut arr) = steps.and_then(|v| v.as_array().cloned()) else {
            return Ok(());
        };
        let target = format!("job:{job_type}");
        let mut changed = false;
        for entry in arr.iter_mut() {
            if entry.get("step").and_then(Value::as_str) == Some(target.as_str())
                && entry.get("status").and_then(Value::as_str) == Some("pending")
            {
                entry["status"] = Value::String(if ok { "ok" } else { "failed" }.into());
                entry["detail"] = Value::String(detail.to_string());
                changed = true;
            }
        }
        if changed {
            let usql = format!(
                "UPDATE itg_receipts SET steps = {} WHERE id = {}",
                crate::db::Driver::ph(1),
                crate::db::Driver::ph(2)
            );
            sqlx::query(crate::db::safe_sql(&usql))
                .bind(Value::Array(arr))
                .bind(*id)
                .execute(&mut *tx)
                .await?;
        }
        Ok(())
    })
}

/// Retry backoff: 10s · 2^(attempt-1), capped at 10 minutes.
#[must_use]
pub fn backoff_secs(attempt: i64) -> i64 {
    let exp = (attempt - 1).clamp(0, 10) as u32;
    (10_i64.saturating_mul(1_i64 << exp)).min(600)
}

/// Persist a failed first pass (stays `received` until the M2 retry runner
/// picks it up; records the failing step timeline for diagnosis).
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn mark_failed_tx(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    envelope: &Value,
    steps: &Value,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE itg_receipts SET envelope = {}, steps = {} WHERE id = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        crate::db::Driver::ph(3)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(envelope)
        .bind(steps)
        .bind(*id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

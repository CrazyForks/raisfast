//! flow_resume model + queries (dev-docs/workflow/await-node.md §3).
//!
//! Claim ledger for parked `await` nodes: park inserts an `open` row, resume
//! closes it via a conditional UPDATE — the single serializer that makes
//! concurrent resumes idempotent (second claimant gets `false` → 409).
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

const FLOW_RESUME_COLS: &str = "id, instance_id, node_id, kind, status, \
     token_hash, token_enc, resume_until, payload, resumed_by, created_at, updated_at";

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct FlowResume {
    pub id: SnowflakeId,
    pub instance_id: SnowflakeId,
    pub node_id: String,
    pub kind: String,
    pub status: String,
    /// Public resume-URL credential (n8n wait webhook / Dify recipient
    /// access_token): sha256 for lookup, AES-GCM ciphertext for display.
    pub token_hash: Option<String>,
    pub token_enc: Option<String>,
    pub resume_until: Option<Timestamp>,
    pub payload: Option<Value>,
    pub resumed_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Ensure an `open` claim row exists for the parked node (idempotent re-park:
/// reuses the existing row instead of duplicating).
pub async fn ensure_open(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
    node_id: &str,
    kind: &str,
    resume_until: Option<Timestamp>,
) -> AppResult<()> {
    if find_open(pool, instance_id, node_id).await?.is_some() {
        return Ok(());
    }
    let sql = format!(
        "INSERT INTO flow_resume (id, instance_id, node_id, kind, status, \
         resume_until, payload, resumed_by, created_at, updated_at) \
         VALUES ({}, {}, {}, {}, 'open', {}, NULL, NULL, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7)
    );
    let now = crate::utils::tz::now_utc();
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*crate::utils::id::new_snowflake_id())
        .bind(*instance_id)
        .bind(node_id)
        .bind(kind)
        .bind(resume_until)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

/// The open claim row for `(instance, node)`, if parked.
pub async fn find_open(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
    node_id: &str,
) -> AppResult<Option<FlowResume>> {
    let sql = format!(
        "SELECT {FLOW_RESUME_COLS} FROM flow_resume \
         WHERE instance_id = {} AND node_id = {} AND status = 'open'",
        Driver::ph(1),
        Driver::ph(2)
    );
    let row = sqlx::query_as::<crate::db::pool::Db, FlowResume>(crate::db::safe_sql(&sql))
        .bind(*instance_id)
        .bind(node_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Attach a public resume token to an open claim (mint-once; hash for lookup,
/// ciphertext for later display in the admin waiting panel).
pub async fn set_token(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    token_hash: &str,
    token_enc: &str,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_resume SET token_hash = {}, token_enc = {}, updated_at = {} \
         WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(token_hash)
        .bind(token_enc)
        .bind(crate::utils::tz::now_utc())
        .bind(*id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Open claim lookup by resume-token hash (public callback endpoint).
pub async fn find_open_by_token_hash(
    pool: &crate::db::Pool,
    token_hash: &str,
) -> AppResult<Option<FlowResume>> {
    let sql = format!(
        "SELECT {FLOW_RESUME_COLS} FROM flow_resume \
         WHERE token_hash = {} AND status = 'open' LIMIT 1",
        Driver::ph(1)
    );
    let row = sqlx::query_as::<crate::db::pool::Db, FlowResume>(crate::db::safe_sql(&sql))
        .bind(token_hash)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Open claims whose `resume_until` has passed (timeout sweeper scan).
pub async fn find_expired_open(
    pool: &crate::db::Pool,
    now: Timestamp,
) -> AppResult<Vec<FlowResume>> {
    let sql = format!(
        "SELECT {FLOW_RESUME_COLS} FROM flow_resume \
         WHERE status = 'open' AND resume_until IS NOT NULL AND resume_until <= {}",
        Driver::ph(1)
    );
    let rows = sqlx::query_as::<crate::db::pool::Db, FlowResume>(crate::db::safe_sql(&sql))
        .bind(now)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Atomically close the open claim with `payload` (the resume envelope) and
/// the acting user. Returns `false` when the row is no longer open — the
/// caller maps that to 409 (already resumed / racing sweeper).
pub async fn claim(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
    node_id: &str,
    payload: &Value,
    resumed_by: Option<SnowflakeId>,
) -> AppResult<bool> {
    let sql = format!(
        "UPDATE flow_resume SET status = 'filled', payload = {}, resumed_by = {}, \
         updated_at = {} WHERE instance_id = {} AND node_id = {} AND status = 'open'",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5)
    );
    let res = sqlx::query(crate::db::safe_sql(&sql))
        .bind(payload)
        .bind(resumed_by)
        .bind(crate::utils::tz::now_utc())
        .bind(*instance_id)
        .bind(node_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn claim_is_single_shot() {
        let pool = crate::test_pool!();
        let iid = SnowflakeId(crate::utils::id::new_id());
        let node = format!("n{}", crate::utils::id::new_id());
        ensure_open(&pool, iid, &node, "human", None).await.unwrap();
        // Second ensure is a no-op (single open row).
        ensure_open(&pool, iid, &node, "human", None).await.unwrap();

        let first = claim(&pool, iid, &node, &json!({"type": "approval"}), None)
            .await
            .unwrap();
        assert!(first, "first claim closes the open row");
        let second = claim(&pool, iid, &node, &json!({"type": "approval"}), None)
            .await
            .unwrap();
        assert!(!second, "second claim loses the race → caller 409");

        let row = find_open(&pool, iid, &node).await.unwrap();
        assert!(row.is_none(), "no open row remains");
    }

    #[tokio::test]
    async fn expired_scan_filters_by_resume_until() {
        let pool = crate::test_pool!();
        let iid = SnowflakeId(crate::utils::id::new_id());
        let past = format!("n{}", crate::utils::id::new_id());
        let future = format!("n{}", crate::utils::id::new_id());
        let now = crate::utils::tz::now_utc();
        ensure_open(
            &pool,
            iid,
            &past,
            "human",
            Some(now - chrono::Duration::seconds(1)),
        )
        .await
        .unwrap();
        ensure_open(
            &pool,
            iid,
            &future,
            "human",
            Some(now + chrono::Duration::seconds(3600)),
        )
        .await
        .unwrap();
        let rows = find_expired_open(&pool, now).await.unwrap();
        assert!(
            rows.iter().all(|r| r.node_id == past),
            "only the overdue row surfaces: {rows:?}"
        );
    }
}

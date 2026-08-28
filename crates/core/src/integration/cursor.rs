//! `itg_channel_cursors` — cursor storage for pull channels (integration.md §3.1.1).
//!
//! One row per channel. Advancement is ALWAYS conditional
//! (`WHERE cursor_value = :expected`) so overlapping cron ticks lose
//! gracefully — the loser abandons its advancement and receipts idempotency
//! absorbs any overlap on the next pull.

use serde_json::Value;

use crate::db::driver::DbDriver;
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// Read the current cursor value (None when the channel never pulled).
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn read(pool: &crate::db::Pool, channel_id: SnowflakeId) -> AppResult<Option<Value>> {
    let sql = format!(
        "SELECT cursor_value FROM itg_channel_cursors WHERE channel_id = {}",
        crate::db::Driver::ph(1)
    );
    let row: Option<(Value,)> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(*channel_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

/// Conditionally advance the cursor: succeeds only when the stored value still
/// equals `expected` (or the row does not exist and `expected` is None).
/// Losers return `false` and abandon — the next pull re-reads.
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn advance(
    pool: &crate::db::Pool,
    channel_id: SnowflakeId,
    expected: Option<&Value>,
    new: &Value,
    now: Timestamp,
) -> AppResult<bool> {
    let result = match expected {
        Some(old) => {
            let sql = format!(
                "UPDATE itg_channel_cursors SET cursor_value = {}, updated_at = {} \
                 WHERE channel_id = {} AND cursor_value = {}",
                crate::db::Driver::ph(1),
                crate::db::Driver::ph(2),
                crate::db::Driver::ph(3),
                crate::db::Driver::cast_json(&crate::db::Driver::ph(4))
            );
            sqlx::query(crate::db::safe_sql(&sql))
                .bind(new)
                .bind(now)
                .bind(*channel_id)
                .bind(old)
                .execute(pool)
                .await?
        }
        None => {
            // First advancement: insert-if-absent (loser abandons on conflict).
            let sql = crate::db::Driver::insert_ignore_sql(
                "itg_channel_cursors",
                "channel_id, cursor_value, updated_at",
                &format!(
                    "{}, {}, {}",
                    crate::db::Driver::ph(1),
                    crate::db::Driver::ph(2),
                    crate::db::Driver::ph(3)
                ),
            );
            sqlx::query(crate::db::safe_sql(&sql))
                .bind(*channel_id)
                .bind(new)
                .bind(now)
                .execute(pool)
                .await?
        }
    };
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cursor_roundtrip_and_conditional_advance() {
        let pool = crate::test_pool!();
        let ch = SnowflakeId::new(crate::utils::id::new_id());
        let now = crate::utils::tz::now_utc();

        // First read: none.
        assert!(read(&pool, ch).await.unwrap().is_none());

        // First advance (insert path).
        let v1 = serde_json::json!({"since_id": "100"});
        assert!(advance(&pool, ch, None, &v1, now).await.unwrap());
        assert_eq!(read(&pool, ch).await.unwrap(), Some(v1.clone()));

        // Correct expectation wins.
        let v2 = serde_json::json!({"since_id": "200"});
        assert!(advance(&pool, ch, Some(&v1), &v2, now).await.unwrap());

        // Stale expectation loses (cursor already at v2).
        let v3 = serde_json::json!({"since_id": "300"});
        assert!(!advance(&pool, ch, Some(&v1), &v3, now).await.unwrap());
        assert_eq!(read(&pool, ch).await.unwrap(), Some(v2));

        // Insert path loses when the row exists.
        assert!(!advance(&pool, ch, None, &v3, now).await.unwrap());
    }
}

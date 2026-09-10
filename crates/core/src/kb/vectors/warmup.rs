//! Bruteforce lazy warm-up (kb-observability-design DR10).
//!
//! `BruteForceIndex` is process-memory: a restart empties it while SQL
//! still holds every embedding — before this, that silently killed dense
//! recall (BM25-only degradation, no error anywhere). The warm-up runs on
//! the first dense recall of a cold KB, single-flight per KB: concurrent
//! first queries do not stampede (losers proceed degraded for that one
//! turn); genuinely-empty KBs are remembered so they are not rescanned
//! every query (negative cache — safe without invalidation because every
//! later write path goes through `vector.upsert`, which warms the index
//! itself). Qdrant persists externally and never warms up.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::{LazyLock, Mutex};

use crate::db::{DbDriver, Driver, Pool};
use crate::kb::vectors::VectorIndex;
use crate::types::snowflake_id::SnowflakeId;

/// KBs already warm-attempted this process (positive + negative cache).
static WARMED: LazyLock<Mutex<HashSet<i64>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// KBs with a rebuild currently in flight.
static INFLIGHT: LazyLock<tokio::sync::Mutex<HashSet<i64>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashSet::new()));

/// Embedded (active) chunk count for one KB in SQL.
async fn sql_embedded_count(pool: &Pool, kb_id: i64) -> crate::errors::app_error::AppResult<i64> {
    let sql = format!(
        "SELECT {} FROM kb_chunks WHERE kb_id = {} AND embedding IS NOT NULL AND status = 'active'",
        Driver::cast_int("COUNT(*)"),
        Driver::ph(1)
    );
    sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql))
        .bind(kb_id)
        .fetch_one(pool)
        .await
        .map_err(|e| crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string())))
}

/// Ensure the in-memory index for `kb_id` is warm before a dense search.
/// Best-effort: errors are warned and the query proceeds (BM25-degraded).
pub async fn ensure_warm(pool: &Pool, vector: &Arc<dyn VectorIndex>, kb_id: i64) {
    if vector.backend_name() != "bruteforce" {
        return; // qdrant persists externally — nothing to warm
    }
    if WARMED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&kb_id)
    {
        return; // already warm (or verified empty) this process
    }
    // Single-flight (DR10): one rebuilder per KB; concurrent first queries
    // proceed degraded rather than waiting (avoids tail latency).
    {
        let mut inflight = INFLIGHT.lock().await;
        if inflight.contains(&kb_id) {
            return;
        }
        inflight.insert(kb_id);
    }
    let result = warm_up(pool, vector, kb_id).await;
    INFLIGHT.lock().await.remove(&kb_id);
    let mut warmed = WARMED.lock().unwrap_or_else(|p| p.into_inner());
    warmed.insert(kb_id);
    if result.is_err() {
        // Not actually warmed on error — allow a later retry.
        warmed.remove(&kb_id);
    }
    if let Err(e) = result {
        tracing::warn!("[kb] lazy warm-up failed for kb {kb_id}: {e}");
    }
}

async fn warm_up(
    pool: &Pool,
    vector: &Arc<dyn VectorIndex>,
    kb_id: i64,
) -> crate::errors::app_error::AppResult<()> {
    if vector.count(kb_id).await? > 0 {
        return Ok(()); // someone warmed it between the check and here
    }
    if sql_embedded_count(pool, kb_id).await? == 0 {
        tracing::debug!("[kb] kb {kb_id} has no embedded chunks — nothing to warm");
        return Ok(()); // negative-cache entry stays
    }
    let n =
        crate::kb::service::rebuild_vector_index_from_sql(pool, vector, SnowflakeId(kb_id)).await?;
    tracing::info!("[kb] lazy warm-up rebuilt kb {kb_id}: {n} units (post-restart cold start)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn warmed_flag_short_circuits() {
        WARMED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(999_999);
        let vector: Arc<dyn VectorIndex> = Arc::new(crate::kb::vectors::BruteForceIndex::new());
        let pool = crate::test_pool!();
        // Returns immediately without touching SQL (flag short-circuit).
        ensure_warm(&pool, &vector, 999_999).await;
        WARMED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&999_999);
    }

    #[tokio::test]
    async fn qdrant_named_backend_never_warms() {
        // Even with a bogus pool, the qdrant branch returns before SQL.
        struct FakeQdrant;
        #[async_trait::async_trait]
        impl VectorIndex for FakeQdrant {
            async fn upsert(
                &self,
                _kb_id: i64,
                _dim: u32,
                _items: &[crate::kb::vectors::VectorItem],
            ) -> crate::errors::app_error::AppResult<()> {
                Ok(())
            }
            async fn delete(
                &self,
                _kb_id: i64,
                _unit_ids: &[i64],
            ) -> crate::errors::app_error::AppResult<()> {
                Ok(())
            }
            async fn delete_all(&self, _kb_id: i64) -> crate::errors::app_error::AppResult<()> {
                Ok(())
            }
            async fn search(
                &self,
                _kb_id: i64,
                _embedding: &[f32],
                _top_k: usize,
                _kind: Option<&str>,
            ) -> crate::errors::app_error::AppResult<Vec<crate::kb::vectors::VectorHit>>
            {
                Ok(Vec::new())
            }
            async fn rebuild(
                &self,
                _kb_id: i64,
                _dim: u32,
                _items: &[crate::kb::vectors::VectorItem],
            ) -> crate::errors::app_error::AppResult<()> {
                Ok(())
            }
            async fn count(&self, _kb_id: i64) -> crate::errors::app_error::AppResult<u64> {
                Ok(0)
            }
            fn backend_name(&self) -> &str {
                "qdrant"
            }
        }
        let pool = crate::test_pool!();
        let backend: Arc<dyn VectorIndex> = Arc::new(FakeQdrant);
        ensure_warm(&pool, &backend, 1).await; // no-op by name
    }
}

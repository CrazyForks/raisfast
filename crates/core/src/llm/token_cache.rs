//! Short-TTL moka cache for sk- token authentication (design §9.1).
//!
//! Sk- auth hashes the bearer key and looks up `llm_tokens` on every relay
//! request. At high single-token QPS that lookup is pure overhead, so the
//! verified row is cached by `key_hash` for a few seconds. Semantics follow
//! new-api `model/token_cache.go`; the implementation reuses the `moka`
//! convention already used by `services::options`. `[照抄语义]`
//!
//! Invalidation is write-path driven: token admin mutations (status / delete /
//! edit) evict immediately. Quota debits do **not** evict — the atomic
//! `UPDATE ... WHERE remain_quota >= ?` in the billing path is authoritative,
//! and the cached quota only feeds the soft "exhausted" flip, so a ≤TTL
//! staleness can never overspend.

use std::time::Duration;

use crate::llm::models::token::LlmToken;
use crate::types::snowflake_id::SnowflakeId;

/// Short TTL (design §9.1): auth is cheap to re-verify, staleness is bounded.
const CACHE_TTL: Duration = Duration::from_secs(5);
const CACHE_MAX_ENTRIES: u64 = 100_000;

fn cache() -> &'static moka::sync::Cache<String, LlmToken> {
    static CACHE: std::sync::OnceLock<moka::sync::Cache<String, LlmToken>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        moka::sync::Cache::builder()
            .max_capacity(CACHE_MAX_ENTRIES)
            .time_to_live(CACHE_TTL)
            // Required by `invalidate_entries_if` (id-based write-path eviction).
            .support_invalidation_closures()
            .build()
    })
}

/// Cached token row for `key_hash`, if fresh.
pub fn get(key_hash: &str) -> Option<LlmToken> {
    cache().get(key_hash)
}

/// Cache a verified token row.
pub fn put(key_hash: &str, token: &LlmToken) {
    cache().insert(key_hash.to_owned(), token.clone());
}

/// Evict one key hash (used when the hash is at hand).
pub fn invalidate_hash(key_hash: &str) {
    cache().invalidate(key_hash);
}

/// Evict every cached entry belonging to `id` (write paths keyed by id).
///
/// `invalidate_entries_if` applies the predicate lazily; the write-back is
/// synchronous enough for the next request in the common case, and the short
/// TTL bounds any lag. The cache is small, so the scan is negligible.
pub fn invalidate_id(id: SnowflakeId) {
    let _ = cache().invalidate_entries_if(move |_, token| token.id == id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::models::token::LlmTokenStatus;
    use crate::types::quota::Quota;

    fn token(id: i64, hash: &str) -> LlmToken {
        LlmToken {
            id: SnowflakeId(id),
            tenant_id: Some("default".to_owned()),
            user_id: SnowflakeId(2),
            name: "t".to_owned(),
            key_hash: hash.to_owned(),
            key_enc: None,
            status: LlmTokenStatus::Enabled,
            remain_quota: Quota(100),
            used_quota: Quota(0),
            unlimited_quota: false,
            expired_at: None,
            allowed_models: None,
            allowed_ips: None,
            token_group: None,
            created_at: crate::utils::tz::now_utc(),
            accessed_at: None,
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    #[test]
    fn put_get_and_invalidate_hash() {
        let hash = format!("h-{}", crate::utils::id::new_id());
        put(&hash, &token(1, &hash));
        assert!(get(&hash).is_some());
        invalidate_hash(&hash);
        assert!(get(&hash).is_none());
    }

    #[test]
    fn invalidate_id_removes_matching_entries() {
        let id = SnowflakeId(crate::utils::id::new_id());
        let h1 = format!("h1-{}", crate::utils::id::new_id());
        let h2 = format!("h2-{}", crate::utils::id::new_id());
        put(&h1, &token(id.0, &h1));
        put(&h2, &token(id.0 + 1, &h2));
        invalidate_id(id);
        // moka applies the predicate lazily; run maintenance to settle it.
        cache().run_pending_tasks();
        assert!(get(&h1).is_none(), "matching id evicted");
        assert!(get(&h2).is_some(), "other id kept");
    }
}

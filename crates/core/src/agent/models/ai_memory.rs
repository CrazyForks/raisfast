//! Agent long-term memory model (`ai_memories`). Ownership scope = (tenant,
//! agent); `superseded_by IS NULL` is the live-row predicate.

use serde::{Deserialize, Serialize};

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

/// Row/column list used by hand-written reads (kept in one place).
const MEMORY_COLS: &str = "id, tenant_id, agent_id, user_id, session_id, mem_key, content, category, \
    importance, superseded_by, pinned, created_at, updated_at";

/// One memory row.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AiMemory {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub agent_id: SnowflakeId,
    pub user_id: Option<SnowflakeId>,
    pub session_id: Option<SnowflakeId>,
    #[sqlx(rename = "mem_key")]
    pub key: String,
    pub content: String,
    pub category: String,
    pub importance: f64,
    pub superseded_by: Option<SnowflakeId>,
    pub pinned: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Find a live row scoped to (agent, user, key).
pub async fn find_memory_by_key(
    pool: &crate::db::Pool,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<AiMemory>> {
    let (clause, _) = scope_clause(tenant_id, user_id, 3);
    let sql = format!(
        "SELECT {MEMORY_COLS} FROM ai_memories WHERE agent_id = {} AND mem_key = {}{clause}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2)
    );
    let mut q = sqlx::query_as::<_, AiMemory>(crate::db::safe_sql(&sql))
        .bind(agent_id)
        .bind(key);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(uid) = user_id {
        q = q.bind(uid);
    }
    Ok(q.fetch_optional(pool).await?)
}

/// Upsert by (tenant, agent, user, key): update the live row content or insert.
#[allow(clippy::too_many_arguments)]
pub async fn store_memory(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    key: &str,
    content: &str,
    category: &str,
    importance: f64,
    pinned: bool,
) -> AppResult<AiMemory> {
    if let Some(existing) = find_memory_by_key(pool, agent_id, user_id, key, tenant_id).await? {
        let now = now_utc();
        let result = raisfast_derive::crud_update!(
            pool,
            "ai_memories",
            bind: ["content" => content, "category" => category, "importance" => importance, "updated_at" => &now],
            where: ("id", existing.id),
            tenant: tenant_id
        )?;
        AppError::expect_affected(&result, "ai_memory")?;
        return Ok(existing);
    }

    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "ai_memories",
        [
            "id" => id,
            "agent_id" => agent_id,
            "user_id" => user_id,
            "session_id" => None::<SnowflakeId>,
            "mem_key" => key,
            "content" => content,
            "category" => category,
            "importance" => importance,
            "pinned" => pinned,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    match find_memory_by_key(pool, agent_id, user_id, key, tenant_id).await? {
        Some(m) => Ok(m),
        None => Err(AppError::not_found("ai_memory")),
    }
}

/// Keyword recall over live rows, scoped to one user. `query = None` returns
/// most recent first.
pub async fn recall_memories(
    pool: &crate::db::Pool,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    tenant_id: Option<&str>,
    query: Option<&str>,
    limit: i64,
) -> AppResult<Vec<AiMemory>> {
    let keyword: Option<String> = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| format!("%{q}%"));

    let mut sql = format!(
        "SELECT {MEMORY_COLS} FROM ai_memories WHERE agent_id = {} AND superseded_by IS NULL",
        crate::db::Driver::ph(1)
    );
    let (tail, mut n) = scope_clause(tenant_id, user_id, 2);
    sql.push_str(&tail);
    if keyword.is_some() {
        sql.push_str(&format!(
            " AND (mem_key LIKE {} OR content LIKE {})",
            crate::db::Driver::ph(n),
            crate::db::Driver::ph(n + 1)
        ));
        n += 2;
    }
    sql.push_str(&format!(
        " ORDER BY updated_at DESC LIMIT {}",
        crate::db::Driver::ph(n)
    ));

    let mut q = sqlx::query_as::<_, AiMemory>(crate::db::safe_sql(&sql)).bind(agent_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(uid) = user_id {
        q = q.bind(uid);
    }
    if let Some(pat) = &keyword {
        q = q.bind(pat.as_str()).bind(pat.as_str());
    }
    Ok(q.bind(limit).fetch_all(pool).await?)
}

/// Delete a memory row scoped to (agent, user, key).
pub async fn forget_memory(
    pool: &crate::db::Pool,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let (clause, _) = scope_clause(tenant_id, user_id, 3);
    let sql = format!(
        "DELETE FROM ai_memories WHERE agent_id = {} AND mem_key = {}{clause}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2)
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql))
        .bind(agent_id)
        .bind(key);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(uid) = user_id {
        q = q.bind(uid);
    }
    Ok(q.execute(pool).await?.rows_affected() > 0)
}

/// Optional scope clause ` AND tenant_id = ? AND user_id = ?` (either/both).
/// Returns the next free placeholder index.
fn scope_clause(
    tenant_id: Option<&str>,
    user_id: Option<SnowflakeId>,
    start_index: usize,
) -> (String, usize) {
    let mut out = String::new();
    let mut n = start_index;
    if tenant_id.is_some() {
        out.push_str(&format!(" AND tenant_id = {}", crate::db::Driver::ph(n)));
        n += 1;
    }
    if user_id.is_some() {
        out.push_str(&format!(" AND user_id = {}", crate::db::Driver::ph(n)));
        n += 1;
    }
    (out, n)
}

/// Memory-tier budget (port of zeroclaw `budget.rs` caps): row and byte caps
/// per category. `0` = unbounded for that cap. Only `core` (rows+bytes) and
/// `daily` (rows) are budget-managed; `conversation`/`custom` never evicted.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryBudgetConfig {
    pub core_max_rows: i64,
    pub core_max_bytes: i64,
    pub daily_max_rows: i64,
}

impl MemoryBudgetConfig {
    fn caps_for(&self, category: &str) -> (i64, i64) {
        match category {
            "core" => (self.core_max_rows, self.core_max_bytes),
            "daily" => (self.daily_max_rows, 0),
            _ => (0, 0),
        }
    }
}

/// How many live rows were evicted (budget report analog of zeroclaw
/// `EvictionReport`, minus the pinned counter — we have no pinned column).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BudgetReport {
    pub evicted_by_count: i64,
    pub evicted_by_bytes: i64,
}

fn live_scope(
    tenant_id: Option<&str>,
    user_id: Option<SnowflakeId>,
    start_index: usize,
) -> (String, usize) {
    let mut out = format!(
        " WHERE category = {} AND agent_id = {} AND superseded_by IS NULL",
        crate::db::Driver::ph(start_index),
        crate::db::Driver::ph(start_index + 1)
    );
    let mut n = start_index + 2;
    if tenant_id.is_some() {
        out.push_str(&format!(" AND tenant_id = {}", crate::db::Driver::ph(n)));
        n += 1;
    }
    if user_id.is_some() {
        out.push_str(&format!(" AND user_id = {}", crate::db::Driver::ph(n)));
        n += 1;
    }
    (out, n)
}

/// Compact one category to its configured budget, scoped to (agent, user).
pub async fn compact_category_to_budget(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    category: &str,
    cfg: MemoryBudgetConfig,
) -> AppResult<BudgetReport> {
    let (max_rows, max_bytes) = cfg.caps_for(category);
    if max_rows <= 0 && max_bytes <= 0 {
        return Ok(BudgetReport::default());
    }
    let mut report = BudgetReport::default();

    // ── row cap ─────────────────────────────────────────────────────────────
    if max_rows > 0 {
        let current = count_live(pool, tenant_id, agent_id, user_id, category).await?;
        if current > max_rows {
            let excess = current - max_rows;
            let ids = evictable_ids(pool, tenant_id, agent_id, user_id, category, excess).await?;
            report.evicted_by_count += delete_ids(pool, &ids).await? as i64;
        }
    }

    // ── byte cap: evict one row at a time until under budget ───────────────
    if max_bytes > 0 {
        loop {
            let current_bytes = live_bytes(pool, tenant_id, agent_id, user_id, category).await?;
            if current_bytes <= max_bytes {
                break;
            }
            let ids = evictable_ids(pool, tenant_id, agent_id, user_id, category, 1).await?;
            if ids.is_empty() {
                break;
            }
            report.evicted_by_bytes += delete_ids(pool, &ids).await? as i64;
        }
    }
    Ok(report)
}

async fn count_live(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    category: &str,
) -> AppResult<i64> {
    let (clause, _) = live_scope(tenant_id, user_id, 1);
    let sql = format!(
        "SELECT {} FROM ai_memories{clause}",
        crate::db::Driver::cast_int("COUNT(*)")
    );
    let mut q = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql)).bind(category);
    q = q.bind(agent_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(uid) = user_id {
        q = q.bind(uid);
    }
    Ok(q.fetch_one(pool).await?)
}

async fn live_bytes(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    category: &str,
) -> AppResult<i64> {
    let (clause, _) = live_scope(tenant_id, user_id, 1);
    let sql = format!(
        "SELECT {} FROM ai_memories{clause}",
        crate::db::Driver::cast_int("COALESCE(SUM(LENGTH(content)), 0)")
    );
    let mut q = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql)).bind(category);
    q = q.bind(agent_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(uid) = user_id {
        q = q.bind(uid);
    }
    Ok(q.fetch_one(pool).await?)
}

/// Lowest-value live rows for (agent, user), up to `limit`.
async fn evictable_ids(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    category: &str,
    limit: i64,
) -> AppResult<Vec<SnowflakeId>> {
    let (clause, n) = live_scope(tenant_id, user_id, 1);
    let limit_ph = crate::db::Driver::ph(n);
    let sql = format!(
        "SELECT id FROM ai_memories{clause} AND pinned = FALSE ORDER BY importance ASC, created_at ASC LIMIT {limit_ph}"
    );
    let mut q = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql)).bind(category);
    q = q.bind(agent_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(uid) = user_id {
        q = q.bind(uid);
    }
    Ok(q.bind(limit)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(SnowflakeId)
        .collect())
}

async fn delete_ids(pool: &crate::db::Pool, ids: &[SnowflakeId]) -> AppResult<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = (0..ids.len())
        .map(|i| crate::db::Driver::ph(i + 1))
        .collect();
    let sql = format!(
        "DELETE FROM ai_memories WHERE id IN ({})",
        placeholders.join(",")
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql));
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

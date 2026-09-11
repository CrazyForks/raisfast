//! `llm_logs` model — usage/billing log rows (design §5.3). Write path is
//! shared by relay (P3) and internal `execute` (P2); P1 ships row + insert.

use serde::{Deserialize, Serialize};

use crate::errors::app_error::AppResult;
use crate::types::quota::Quota;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

define_enum!(
    LogSource {
        Relay = "relay",
        Agent = "agent",
        Kb = "kb",
        Flow = "flow",
        Test = "test",
    }
);

/// One usage-log row (`quota` stays 0 for internal calls, design §9.3).
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LlmLog {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub request_id: Option<String>,
    pub user_id: Option<SnowflakeId>,
    pub token_id: Option<SnowflakeId>,
    pub source: LogSource,
    pub channel_id: Option<SnowflakeId>,
    pub key_index: Option<i32>,
    pub model_name: String,
    pub is_stream: bool,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_write_tokens: i32,
    pub quota: Quota,
    pub cost_quota: Quota,
    pub detail: Option<serde_json::Value>,
    pub elapsed_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error_message: Option<String>,
    /// UTC business day (`YYYY-MM-DD`) fixed at insert — deterministic,
    /// indexable grouping key for daily stats (see `daily_stats`).
    pub day: String,
    pub created_at: Timestamp,
}

/// Payload for inserting a log row (best-effort: caller logs failures).
#[derive(Debug)]
pub struct NewLog {
    pub tenant_id: Option<String>,
    pub request_id: Option<String>,
    pub user_id: Option<SnowflakeId>,
    pub token_id: Option<SnowflakeId>,
    pub source: LogSource,
    pub channel_id: Option<SnowflakeId>,
    pub key_index: Option<i32>,
    pub model_name: String,
    pub is_stream: bool,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_write_tokens: i32,
    pub quota: Quota,
    pub cost_quota: Quota,
    pub detail: Option<serde_json::Value>,
    pub elapsed_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error_message: Option<String>,
    /// Optional explicit business day; `insert_log` defaults to today (UTC).
    pub day: Option<String>,
}

impl Default for NewLog {
    fn default() -> Self {
        Self {
            tenant_id: None,
            request_id: None,
            user_id: None,
            token_id: None,
            source: LogSource::Relay,
            channel_id: None,
            key_index: None,
            model_name: String::new(),
            is_stream: false,
            prompt_tokens: 0,
            completion_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            quota: Quota::default(),
            cost_quota: Quota::default(),
            detail: None,
            elapsed_ms: None,
            status_code: None,
            error_message: None,
            day: None,
        }
    }
}

/// Insert a usage-log row. The business `day` is fixed here (UTC) unless the
/// caller provided one explicitly.
pub async fn insert_log(pool: &crate::db::Pool, l: NewLog) -> AppResult<()> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    let day = l.day.unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    raisfast_derive::crud_insert!(
        pool,
        "llm_logs",
        [
            "id" => id,
            "request_id" => l.request_id,
            "user_id" => l.user_id,
            "token_id" => l.token_id,
            "source" => l.source.as_str(),
            "channel_id" => l.channel_id,
            "key_index" => l.key_index,
            "model_name" => l.model_name,
            "is_stream" => l.is_stream,
            "prompt_tokens" => l.prompt_tokens,
            "completion_tokens" => l.completion_tokens,
            "cache_read_tokens" => l.cache_read_tokens,
            "cache_write_tokens" => l.cache_write_tokens,
            "quota" => l.quota,
            "cost_quota" => l.cost_quota,
            "detail" => l.detail,
            "elapsed_ms" => l.elapsed_ms,
            "status_code" => l.status_code,
            "error_message" => l.error_message,
            "day" => day,
            "created_at" => &now
        ],
        tenant: l.tenant_id.as_deref()
    )?;
    Ok(())
}

/// Admin log query filters (§12).
#[derive(Debug, Default, serde::Deserialize)]
pub struct LogFilters {
    pub channel_id: Option<String>,
    pub token_id: Option<String>,
    pub model_name: Option<String>,
    /// Resolved owner ids (username search; empty + `username_given` = no match).
    pub user_ids: Vec<SnowflakeId>,
    /// True when a username filter was requested (drives the empty-match case).
    pub username_given: bool,
}

/// One day's aggregate over `llm_logs` (amounts in raw quota units).
#[derive(Debug, Clone, Serialize)]
pub struct DailyStat {
    pub date: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub quota: i64,
    pub cost_quota: i64,
}

/// Daily aggregates over the last `days` days (inclusive of today).
pub async fn daily_stats(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    days: i64,
) -> AppResult<Vec<DailyStat>> {
    let days = days.clamp(1, 365);
    let end = crate::utils::tz::now_utc().date_naive();
    let start = end - chrono::Duration::days(days - 1);
    let buckets = stats_by(
        pool,
        tenant_id,
        "day",
        &start.format("%Y-%m-%d").to_string(),
        &end.format("%Y-%m-%d").to_string(),
    )
    .await?;
    Ok(buckets
        .into_iter()
        .map(|b| DailyStat {
            date: b.key,
            requests: b.requests,
            prompt_tokens: b.prompt_tokens,
            completion_tokens: b.completion_tokens,
            quota: b.quota,
            cost_quota: b.cost_quota,
        })
        .collect())
}

/// One aggregate bucket for any grouping dimension (amounts in raw quota).
#[derive(Debug, Clone, Serialize)]
pub struct StatBucket {
    /// Stable group key (`YYYY-MM-DD` / model name / user id / channel id).
    pub key: String,
    /// Human label (username / channel name where available, else the key).
    pub label: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub quota: i64,
    pub cost_quota: i64,
}

/// Group `llm_logs` by `group_by` ∈ {`day`, `model`, `user`, `channel`} within
/// the inclusive `[start, end]` business-day window (both `YYYY-MM-DD`).
///
/// `day`/`model` need no join; `user`/`channel` LEFT JOIN their dimension
/// tables for a display label (falling back to the raw id). Ids are read as
/// integers and stringified in Rust to stay dialect-agnostic (no int→text
/// CAST). Rows come back ordered: `day` ascending, others by charge DESC.
pub async fn stats_by(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    group_by: &str,
    start: &str,
    end: &str,
) -> AppResult<Vec<StatBucket>> {
    use crate::db::driver::DbDriver;
    use sqlx::Row;

    let requests_expr = crate::db::Driver::cast_int("COUNT(*)");
    let prompt_expr = crate::db::Driver::cast_int("COALESCE(SUM(l.prompt_tokens), 0)");
    let completion_expr = crate::db::Driver::cast_int("COALESCE(SUM(l.completion_tokens), 0)");
    let quota_expr = crate::db::Driver::cast_int("COALESCE(SUM(l.quota), 0)");
    let cost_expr = crate::db::Driver::cast_int("COALESCE(SUM(l.cost_quota), 0)");

    let (key_expr, label_expr, join, group_expr, order_expr, key_is_id) = match group_by {
        "model" => (
            "l.model_name",
            "l.model_name",
            "",
            "l.model_name",
            "SUM(l.quota) DESC",
            false,
        ),
        "user" => (
            "l.user_id",
            "u.username",
            "LEFT JOIN users u ON u.id = l.user_id",
            "l.user_id, u.username",
            "SUM(l.quota) DESC",
            true,
        ),
        "channel" => (
            "l.channel_id",
            "c.name",
            "LEFT JOIN llm_channels c ON c.id = l.channel_id",
            "l.channel_id, c.name",
            "SUM(l.quota) DESC",
            true,
        ),
        _ => ("l.day", "l.day", "", "l.day", "l.day ASC", false),
    };

    let tenant = if tenant_id.is_some() {
        format!(" AND l.tenant_id = {}", crate::db::Driver::ph(3))
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT {key_expr} AS k, {label_expr} AS label, {requests_expr} AS requests, \
         {prompt_expr} AS prompt_tokens, {completion_expr} AS completion_tokens, \
         {quota_expr} AS quota, {cost_expr} AS cost_quota \
         FROM llm_logs l {join} \
         WHERE l.day BETWEEN {} AND {}{tenant} \
         GROUP BY {group_expr} ORDER BY {order_expr}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2)
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql)).bind(start).bind(end);
    if tenant_id.is_some() {
        q = q.bind(crate::db::tenant::resolve_tenant(tenant_id));
    }
    let rows = q.fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let key = if key_is_id {
            row.try_get::<Option<i64>, _>("k")?
                .map(|v| v.to_string())
                .unwrap_or_default()
        } else {
            row.try_get::<Option<String>, _>("k")?.unwrap_or_default()
        };
        let label = if key_is_id {
            row.try_get::<Option<String>, _>("label")?
                .unwrap_or_else(|| key.clone())
        } else {
            key.clone()
        };
        out.push(StatBucket {
            key,
            label,
            requests: row.try_get("requests")?,
            prompt_tokens: row.try_get("prompt_tokens")?,
            completion_tokens: row.try_get("completion_tokens")?,
            quota: row.try_get("quota")?,
            cost_quota: row.try_get("cost_quota")?,
        });
    }
    Ok(out)
}

/// Paged admin log query. Hand-written dynamic SQL (stats_by precedent):
/// the optional-equality macro form can't express the `user_id IN (...)`
/// behind the username search. Conditions and binds are built in the same
/// fixed order so placeholder numbering stays consistent across dialects.
pub async fn query_paged(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    filters: &LogFilters,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<LlmLog>, i64)> {
    use crate::db::driver::DbDriver;
    use crate::types::snowflake_id::{SnowflakeId, parse_id};
    let channel: Option<SnowflakeId> =
        filters.channel_id.as_deref().and_then(|s| parse_id(s).ok());
    let token: Option<SnowflakeId> = filters.token_id.as_deref().and_then(|s| parse_id(s).ok());
    let model = filters.model_name.clone().filter(|m| !m.is_empty());
    let user_ids: &[SnowflakeId] = &filters.user_ids;
    // Username searched but no user matched → definitively empty page.
    if filters.username_given && user_ids.is_empty() {
        return Ok((Vec::new(), 0));
    }

    let ph = crate::db::Driver::ph;
    let mut idx = 1usize;
    let mut conds = String::new();
    if channel.is_some() {
        conds.push_str(&format!(" AND channel_id = {}", ph(idx)));
        idx += 1;
    }
    if token.is_some() {
        conds.push_str(&format!(" AND token_id = {}", ph(idx)));
        idx += 1;
    }
    if model.is_some() {
        conds.push_str(&format!(" AND model_name = {}", ph(idx)));
        idx += 1;
    }
    if !user_ids.is_empty() {
        let list = (0..user_ids.len())
            .map(|i| ph(idx + i))
            .collect::<Vec<_>>()
            .join(", ");
        conds.push_str(&format!(" AND user_id IN ({list})"));
        idx += user_ids.len();
    }
    let tenant = if tenant_id.is_some() {
        let frag = format!(" AND tenant_id = {}", ph(idx));
        idx += 1;
        frag
    } else {
        String::new()
    };

    let cols = "id, tenant_id, request_id, user_id, token_id, source, channel_id, \
                key_index, model_name, is_stream, prompt_tokens, completion_tokens, \
                cache_read_tokens, cache_write_tokens, quota, cost_quota, detail, \
                elapsed_ms, status_code, error_message, day, created_at";
    let offset = (page - 1).max(0) * page_size;
    let data_sql = format!(
        "SELECT {cols} FROM llm_logs WHERE 1=1{conds}{tenant} \
         ORDER BY id DESC LIMIT {lim} OFFSET {off}",
        lim = ph(idx),
        off = ph(idx + 1)
    );
    let count_expr = crate::db::Driver::cast_int("COUNT(*)");
    let count_sql = format!("SELECT {count_expr} FROM llm_logs WHERE 1=1{conds}{tenant}");

    let mut dq = sqlx::query_as::<crate::db::pool::Db, LlmLog>(crate::db::safe_sql(&data_sql));
    let mut cq = sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&count_sql));
    if let Some(c) = channel {
        dq = dq.bind(c);
        cq = cq.bind(c);
    }
    if let Some(t) = token {
        dq = dq.bind(t);
        cq = cq.bind(t);
    }
    if let Some(m) = &model {
        dq = dq.bind(m);
        cq = cq.bind(m);
    }
    for id in user_ids {
        dq = dq.bind(*id);
        cq = cq.bind(*id);
    }
    if tenant_id.is_some() {
        let tv = crate::db::tenant::resolve_tenant(tenant_id);
        dq = dq.bind(tv);
        cq = cq.bind(tv);
    }
    dq = dq.bind(page_size).bind(offset);
    let data = dq.fetch_all(pool).await?;
    let total = cq.fetch_one(pool).await?;
    Ok((data, total))
}

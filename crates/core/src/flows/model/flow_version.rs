//! flow_version model + queries (dev-docs/workflow db-schema.md).
//! Immutable definition snapshots; publish = append.
use serde::Serialize;
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

const FLOW_VERSION_COLS: &str = "id, flow_id, version_number, definition, created_by, created_at";

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct FlowVersion {
    pub id: SnowflakeId,
    pub flow_id: SnowflakeId,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version_number: i64,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub definition: Value,
    pub created_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
}

pub async fn insert_flow_version(pool: &crate::db::Pool, v: &FlowVersion) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO flow_version ({FLOW_VERSION_COLS}) VALUES ({}, {}, {}, {}, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*v.id)
        .bind(*v.flow_id)
        .bind(v.version_number)
        .bind(&v.definition)
        .bind(v.created_by)
        .bind(v.created_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// Latest published version of a flow (None = not published yet).
pub async fn latest_version(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
) -> AppResult<Option<FlowVersion>> {
    let sql = format!(
        "SELECT {FLOW_VERSION_COLS} FROM flow_version WHERE flow_id = {} \
         ORDER BY version_number DESC LIMIT 1",
        Driver::ph(1)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, FlowVersion>(crate::db::safe_sql(&sql))
            .bind(*flow_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Every version of a flow, oldest first.
pub async fn list_versions(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
) -> AppResult<Vec<FlowVersion>> {
    let sql = format!(
        "SELECT {FLOW_VERSION_COLS} FROM flow_version WHERE flow_id = {} \
         ORDER BY version_number ASC",
        Driver::ph(1)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, FlowVersion>(crate::db::safe_sql(&sql))
            .bind(*flow_id)
            .fetch_all(pool)
            .await?,
    )
}

/// The highest published `version_number` of a flow (fallback for the list
/// view when `current_version` metadata is missing/stale).
pub async fn current_version_number(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
) -> AppResult<Option<i64>> {
    let sql = format!(
        "SELECT {} FROM flow_version WHERE flow_id = {} \
         ORDER BY version_number DESC LIMIT 1",
        Driver::cast_int("version_number"),
        Driver::ph(1)
    );
    Ok(
        sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&sql))
            .bind(*flow_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn find_version_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<Option<FlowVersion>> {
    let sql = format!(
        "SELECT {FLOW_VERSION_COLS} FROM flow_version WHERE id = {}",
        Driver::ph(1)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, FlowVersion>(crate::db::safe_sql(&sql))
            .bind(*id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Rewrite `"type": "llm"` node data to `"chat"` inside a definition/draft
/// JSON value (recursive — covers iteration bodies). Returns whether
/// anything changed (media-nodes.md §1.2 one-time migration).
fn rewrite_llm_node_kind(value: &mut Value) -> bool {
    let mut changed = false;
    match value {
        Value::Object(map) => {
            if let Some(data) = map.get_mut("data")
                && let Some(t) = data.get_mut("type")
                && t.as_str() == Some("llm")
            {
                *t = Value::String("chat".into());
                changed = true;
            }
            for (_, v) in map.iter_mut() {
                changed |= rewrite_llm_node_kind(v);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                changed |= rewrite_llm_node_kind(item);
            }
        }
        _ => {}
    }
    changed
}

/// One-time startup migration (media-nodes.md §1.2): rewrite stored `"llm"`
/// node kinds to `"chat"` in every published `flow_version.definition` and
/// every working draft in `flow.extra._draft`. The graph loader keeps a
/// permanent read alias, so this is canonicalization (canvas shows `chat`,
/// republish writes clean JSON) rather than a correctness requirement —
/// leftovers from backup restores are still executed fine.
///
/// # Errors
///
/// Propagates DB errors; per-row failures abort the run (safe to re-run —
/// the rewrite is idempotent).
pub async fn migrate_llm_node_kind(pool: &crate::db::Pool) -> AppResult<u64> {
    const NEEDLE: &str = "\"type\":\"llm\"";
    let mut migrated: u64 = 0;

    // Published versions.
    let needle = format!("%{NEEDLE}%");
    let version_ids: Vec<i64> =
        sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&format!(
            "SELECT id FROM flow_version WHERE definition LIKE {}",
            Driver::ph(1)
        )))
        .bind(needle.clone())
        .fetch_all(pool)
        .await?;
    for vid in version_ids {
        let Some(mut version) = find_version_by_id(pool, SnowflakeId(vid)).await? else {
            continue;
        };
        if rewrite_llm_node_kind(&mut version.definition) {
            let update = format!(
                "UPDATE flow_version SET definition = {} WHERE id = {}",
                Driver::ph(1),
                Driver::ph(2)
            );
            sqlx::query(crate::db::safe_sql(&update))
                .bind(&version.definition)
                .bind(vid)
                .execute(pool)
                .await?;
            migrated += 1;
        }
    }

    // Working drafts (flow.extra._draft).
    let draft_sql = format!("SELECT id FROM flow WHERE extra LIKE {}", Driver::ph(1));
    let flow_ids: Vec<i64> =
        sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&draft_sql))
            .bind(format!("%{NEEDLE}%"))
            .fetch_all(pool)
            .await?;
    for fid in flow_ids {
        let flow = super::flow::find_flow_by_id(pool, SnowflakeId(fid)).await?;
        let Some(mut draft) = super::flow::flow_draft(&flow) else {
            continue;
        };
        if rewrite_llm_node_kind(&mut draft) {
            super::flow::set_flow_draft(pool, SnowflakeId(fid), Some(draft)).await?;
            migrated += 1;
        }
    }
    Ok(migrated)
}

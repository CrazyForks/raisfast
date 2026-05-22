use crate::types::snowflake_id::SnowflakeId;
use serde::Serialize;
use serde_json::Value;

use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::models::content_revision::{self, ContentRevision, RevisionSummary};

#[derive(Debug, Serialize)]
pub struct DiffResult {
    pub revision_a: ContentRevision,
    pub revision_b: ContentRevision,
    pub diff: Value,
}

pub async fn list_revisions(
    pool: &Pool,
    content_type: &str,
    content_id: SnowflakeId,
    _page: i64,
    _page_size: i64,
) -> AppResult<(Vec<RevisionSummary>, i64)> {
    let items = content_revision::list_revisions(pool, content_type, content_id).await?;
    let total = items.len() as i64;
    let summaries = items
        .into_iter()
        .map(|r| RevisionSummary {
            id: r.id,
            revision_number: r.revision_number,
            created_by: r.created_by,
            created_at: r.created_at,
        })
        .collect();
    Ok((summaries, total))
}

pub async fn get_revision(
    pool: &Pool,
    content_type: &str,
    content_id: SnowflakeId,
    revision_id: SnowflakeId,
) -> AppResult<ContentRevision> {
    content_revision::get_revision(pool, content_type, content_id, revision_id)
        .await?
        .ok_or_else(|| AppError::not_found("revision"))
}

pub async fn restore_revision(
    pool: &Pool,
    content_type: &str,
    content_id: SnowflakeId,
    revision_id: SnowflakeId,
) -> AppResult<Value> {
    let revision = get_revision(pool, content_type, content_id, revision_id).await?;
    let mut snapshot: Value =
        serde_json::from_str(&revision.snapshot).map_err(|e| AppError::Internal(e.into()))?;
    if let Some(obj) = snapshot.as_object_mut() {
        obj.remove(crate::constants::COL_ID);
        obj.remove("created_at");
        obj.remove("updated_at");
    }
    Ok(snapshot)
}

pub async fn diff_revisions(
    pool: &Pool,
    content_type: &str,
    content_id: SnowflakeId,
    rev_id_a: i64,
    rev_id_b: i64,
) -> AppResult<DiffResult> {
    let a = get_revision(pool, content_type, content_id, SnowflakeId(rev_id_a)).await?;
    let b = get_revision(pool, content_type, content_id, SnowflakeId(rev_id_b)).await?;
    let snap_a: Value =
        serde_json::from_str(&a.snapshot).map_err(|e| AppError::Internal(e.into()))?;
    let snap_b: Value =
        serde_json::from_str(&b.snapshot).map_err(|e| AppError::Internal(e.into()))?;
    let diff = content_revision::compute_diff(&snap_a, &snap_b);
    Ok(DiffResult {
        revision_a: a,
        revision_b: b,
        diff,
    })
}

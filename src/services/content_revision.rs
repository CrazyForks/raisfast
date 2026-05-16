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
    content_id: &str,
    _page: i64,
    _page_size: i64,
) -> AppResult<(Vec<RevisionSummary>, i64)> {
    let items = content_revision::list_revisions(pool, content_type, content_id).await?;
    let total = items.len() as i64;
    Ok((items, total))
}

pub async fn get_revision(
    pool: &Pool,
    content_type: &str,
    content_id: &str,
    revision_id: i64,
) -> AppResult<ContentRevision> {
    content_revision::get_revision(pool, content_type, content_id, revision_id)
        .await?
        .ok_or_else(|| AppError::not_found("revision"))
}

pub async fn restore_revision(
    pool: &Pool,
    content_type: &str,
    content_id: &str,
    revision_id: i64,
) -> AppResult<Value> {
    let revision = get_revision(pool, content_type, content_id, revision_id).await?;
    let mut snapshot: Value = serde_json::from_str(&revision.snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot parse: {e}")))?;
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
    content_id: &str,
    rev_id_a: i64,
    rev_id_b: i64,
) -> AppResult<DiffResult> {
    let a = get_revision(pool, content_type, content_id, rev_id_a).await?;
    let b = get_revision(pool, content_type, content_id, rev_id_b).await?;
    let snap_a: Value = serde_json::from_str(&a.snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot A parse: {e}")))?;
    let snap_b: Value = serde_json::from_str(&b.snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot B parse: {e}")))?;
    let diff = content_revision::compute_diff(&snap_a, &snap_b);
    Ok(DiffResult {
        revision_a: a,
        revision_b: b,
        diff,
    })
}

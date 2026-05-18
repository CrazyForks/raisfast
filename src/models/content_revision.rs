//! Content revision history model and database queries
//!
//! Saves a snapshot for each update of content types with `versioning` enabled.
//! Supports: listing history, retrieving snapshots, rolling back to a specific version,
//! and computing diffs between two versions.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ContentRevision {
    pub id: i64,
    pub document_id: String,
    pub content_type: String,
    pub record_id: String,
    pub revision_number: i64,
    pub snapshot: String,
    pub created_by: Option<i64>,
    pub created_at: Timestamp,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize)]
pub struct RevisionSummary {
    pub id: i64,
    pub revision_number: i64,
    pub created_by: Option<i64>,
    pub created_at: Timestamp,
}

pub async fn create_revision(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
    snapshot: &Value,
    created_by: Option<i64>,
) -> AppResult<ContentRevision> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_utc();

    let next_rev = next_revision_number(pool, content_type, record_id).await?;

    let snapshot_str = serde_json::to_string(snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot serialize: {e}")))?;

    raisfast_derive::crud_insert!(pool, "content_revisions", [
        "document_id" => &document_id,
        "content_type" => content_type,
        "record_id" => record_id,
        "revision_number" => next_rev,
        "snapshot" => &snapshot_str,
        "created_by" => created_by,
        "created_at" => now,
    ])?;

    Ok(
        raisfast_derive::crud_find_one!(pool, "content_revisions", ContentRevision, "document_id" => &document_id)?,
    )
}

async fn next_revision_number(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
) -> AppResult<i64> {
    raisfast_derive::check_schema!(
        "content_revisions",
        "revision_number",
        "content_type",
        "record_id"
    );
    let sql = format!(
        "SELECT COALESCE(MAX(revision_number), 0) FROM content_revisions WHERE content_type = {} AND record_id = {}",
        ph(1),
        ph(2),
    );
    let max: i64 = sqlx::query_scalar(&sql)
        .bind(content_type)
        .bind(record_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("max rev: {e}")))?;
    Ok(max + 1)
}

pub async fn list_revisions(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
) -> AppResult<Vec<ContentRevision>> {
    Ok(
        raisfast_derive::crud_find_all!(pool, "content_revisions", ContentRevision, "content_type" => content_type, and: ["record_id" => record_id], order_by: "revision_number DESC")?,
    )
}

pub async fn get_revision(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
    revision_id: i64,
) -> AppResult<Option<ContentRevision>> {
    Ok(
        raisfast_derive::crud_find!(pool, "content_revisions", ContentRevision, "id" => revision_id, and: ["content_type" => content_type, "record_id" => record_id])?,
    )
}

pub fn compute_diff(old: &Value, new: &Value) -> Value {
    let old_obj = old.as_object();
    let new_obj = new.as_object();

    match (old_obj, new_obj) {
        (Some(old_map), Some(new_map)) => {
            let mut added = serde_json::Map::new();
            let mut removed = serde_json::Map::new();
            let mut changed = serde_json::Map::new();

            for (k, v) in new_map {
                match old_map.get(k) {
                    None => {
                        added.insert(k.clone(), v.clone());
                    }
                    Some(old_v) if old_v != v => {
                        changed.insert(k.clone(), serde_json::json!({"from": old_v, "to": v}));
                    }
                    _ => {}
                }
            }

            for (k, v) in old_map {
                if !new_map.contains_key(k) {
                    removed.insert(k.clone(), v.clone());
                }
            }

            serde_json::json!({
                "added": added,
                "removed": removed,
                "changed": changed,
            })
        }
        _ => serde_json::json!({
            "old": old,
            "new": new,
        }),
    }
}

pub async fn delete_revisions(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
) -> AppResult<u64> {
    let result = raisfast_derive::crud_delete!(pool, "content_revisions", "content_type" => content_type, and: ["record_id" => record_id])?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_diff_basic() {
        let old = serde_json::json!({
            "title": "Hello",
            "content": "World",
            "status": "draft"
        });
        let new = serde_json::json!({
            "title": "Hello Updated",
            "content": "World",
            "status": "published",
            "excerpt": "New field"
        });

        let diff = compute_diff(&old, &new);

        let changed = diff.get("changed").unwrap().as_object().unwrap();
        assert_eq!(changed.len(), 2);
        assert!(changed.contains_key("title"));
        assert!(changed.contains_key("status"));

        let added = diff.get("added").unwrap().as_object().unwrap();
        assert_eq!(added.len(), 1);
        assert!(added.contains_key("excerpt"));

        let removed = diff.get("removed").unwrap().as_object().unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn compute_diff_removed_field() {
        let old = serde_json::json!({"a": 1, "b": 2});
        let new = serde_json::json!({"a": 1});

        let diff = compute_diff(&old, &new);
        let removed = diff.get("removed").unwrap().as_object().unwrap();
        assert_eq!(removed.len(), 1);
        assert!(removed.contains_key("b"));
    }

    #[test]
    fn compute_diff_no_changes() {
        let old = serde_json::json!({"a": 1});
        let new = serde_json::json!({"a": 1});

        let diff = compute_diff(&old, &new);
        let changed = diff.get("changed").unwrap().as_object().unwrap();
        assert!(changed.is_empty());
    }
}

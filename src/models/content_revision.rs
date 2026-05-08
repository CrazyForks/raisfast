//! 内容版本历史模型与数据库查询
//!
//! 为启用 `versioning` 的 content type 保存每次更新的快照。
//! 支持：列出历史、获取快照、回滚到指定版本、两个版本间 diff。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};

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
    pub created_at: String,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize)]
pub struct RevisionSummary {
    pub id: i64,
    pub revision_number: i64,
    pub created_by: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, FromRow)]
struct RevisionSummaryRow {
    id: i64,
    revision_number: i64,
    created_by: Option<i64>,
    created_at: String,
}

pub async fn create_revision(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
    snapshot: &Value,
    created_by: Option<&str>,
) -> AppResult<ContentRevision> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_str();

    let next_rev = next_revision_number(pool, content_type, record_id).await?;

    let snapshot_str = serde_json::to_string(snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot serialize: {e}")))?;

    let sql = format!(
        "INSERT INTO content_revisions (document_id, content_type, record_id, revision_number, snapshot, created_by, created_at) VALUES ({}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(content_type)
        .bind(record_id)
        .bind(next_rev)
        .bind(&snapshot_str)
        .bind(created_by)
        .bind(&now)
        .execute(pool)
        .await?;

    let sql = format!(
        "SELECT * FROM content_revisions WHERE document_id = {}",
        ph(1)
    );
    let row = sqlx::query_as::<_, ContentRevision>(&sql)
        .bind(&document_id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

async fn next_revision_number(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
) -> AppResult<i64> {
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
) -> AppResult<Vec<RevisionSummary>> {
    let sql = format!(
        "SELECT id, revision_number, created_by, created_at FROM content_revisions WHERE content_type = {} AND record_id = {} ORDER BY revision_number DESC",
        ph(1),
        ph(2),
    );
    let rows = sqlx::query_as::<_, RevisionSummaryRow>(&sql)
        .bind(content_type)
        .bind(record_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| RevisionSummary {
            id: r.id,
            revision_number: r.revision_number,
            created_by: r.created_by,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn get_revision(
    pool: &crate::db::Pool,
    content_type: &str,
    record_id: &str,
    revision_id: i64,
) -> AppResult<Option<ContentRevision>> {
    let sql = format!(
        "SELECT * FROM content_revisions WHERE id = {} AND content_type = {} AND record_id = {}",
        ph(1),
        ph(2),
        ph(3),
    );
    let row = sqlx::query_as::<_, ContentRevision>(&sql)
        .bind(revision_id)
        .bind(content_type)
        .bind(record_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
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
    let sql = format!(
        "DELETE FROM content_revisions WHERE content_type = {} AND record_id = {}",
        ph(1),
        ph(2),
    );
    let result = sqlx::query(&sql)
        .bind(content_type)
        .bind(record_id)
        .execute(pool)
        .await?;
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

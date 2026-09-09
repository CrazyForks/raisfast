//! `kb_documents` table — document row model, ingest status machine
//! (`pending → parsing → chunking → embedding → indexing → ready | failed`,
//! [抄WK:types/knowledge_process.go]) and queries. The `steps` JSON column
//! records per-step `{start, end}` timestamps for admin progress display.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// Ordered pipeline steps (excludes terminal `ready`/`failed`).
pub const PIPELINE_STEPS: [&str; 4] = ["parsing", "chunking", "embedding", "indexing"];

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbDocument {
    pub id: SnowflakeId,
    pub kb_id: SnowflakeId,
    pub tenant_id: String,
    pub title: String,
    /// `upload` | `online` | `url` (D7: v1 ships upload + online).
    pub source: String,
    pub storage_key: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
    pub parse_format: String,
    /// `pending` | `parsing` | `chunking` | `embedding` | `indexing`
    /// | `ready` | `failed` [抄WK:types/knowledge_process.go].
    pub status: String,
    pub error: Option<String>,
    pub chunk_count: i64,
    /// Per-step timing: `{"parsing": {"start": ts, "end": ts}, ...}`.
    #[sqlx(default)]
    pub steps: Option<Value>,
    pub created_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Create-document command (repo `commands::CreateMediaCmd` pattern [抄RF]).
#[derive(Debug, Clone)]
pub struct CreateKbDocumentCmd {
    pub kb_id: SnowflakeId,
    pub title: String,
    pub source: String,
    pub storage_key: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
    pub created_by: Option<SnowflakeId>,
}

pub async fn create_document(
    pool: &crate::db::Pool,
    cmd: &CreateKbDocumentCmd,
    tenant_id: &str,
) -> AppResult<KbDocument> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_documents",
        [
            "id" => id,
            "kb_id" => cmd.kb_id,
            "tenant_id" => tenant_id,
            "title" => cmd.title.as_str(),
            "source" => cmd.source.as_str(),
            "storage_key" => cmd.storage_key.as_deref(),
            "mime_type" => cmd.mime_type.as_deref(),
            "size" => cmd.size,
            "created_by" => cmd.created_by,
            "created_at" => now
        ],
        tenant: Some(tenant_id)
    )?;
    find_document_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("document insert returned no row")))
}

pub async fn find_document_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Option<KbDocument>> {
    Ok(raisfast_derive::crud_find!(
        pool,
        "kb_documents",
        KbDocument,
        where: ("id", id),
        tenant: Some(tenant_id)
    )?)
}

pub async fn list_documents(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
    status: Option<&str>,
    page: i64,
    page_size: i64,
    tenant_id: &str,
) -> AppResult<(Vec<KbDocument>, i64)> {
    match status {
        Some(s) => Ok(raisfast_derive::crud_query_paged!(
            pool,
            KbDocument,
            table: "kb_documents",
            where: AND(("kb_id", kb_id), ("status", s)),
            order_by: "created_at DESC",
            tenant: Some(tenant_id),
            page: page,
            page_size: page_size
        )),
        None => Ok(raisfast_derive::crud_query_paged!(
            pool,
            KbDocument,
            table: "kb_documents",
            where: ("kb_id", kb_id),
            order_by: "created_at DESC",
            tenant: Some(tenant_id),
            page: page,
            page_size: page_size
        )),
    }
}

/// Transition the status machine and maintain the `steps` timing map:
/// entering the first step (`parsing`) resets the map (fresh re-run);
/// each subsequent transition closes the previous step's `end` and opens
/// the new step's `start`. Terminal `ready`/`failed` just close the last
/// step. Timestamps are RFC3339 strings.
pub async fn set_document_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: &str,
    error: Option<&str>,
    chunk_count: Option<i64>,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let now_str = now.to_rfc3339();

    // Read current row to close the previous step (status + steps map).
    let current = find_document_by_id(pool, id, tenant_id).await?;
    let steps = match (&current, status) {
        (Some(_), "parsing") => serde_json::json!({ "parsing": { "start": now_str } }),
        (Some(cur), s) => {
            let mut map = cur.steps.clone().unwrap_or_else(|| serde_json::json!({}));
            // Close the previous pipeline step if it is still open.
            if PIPELINE_STEPS.contains(&cur.status.as_str())
                && map.get(&cur.status).is_some_and(|v| v.get("end").is_none())
                && let Some(step) = map.get_mut(cur.status.as_str())
            {
                step["end"] = Value::String(now_str.clone());
            }
            // Open the new step (non-terminal transitions only).
            if PIPELINE_STEPS.contains(&s) {
                map[s] = serde_json::json!({ "start": now_str });
            }
            map
        }
        (None, _) => serde_json::json!({}),
    };

    if let Some(n) = chunk_count {
        raisfast_derive::crud_update!(
            pool,
            "kb_documents",
            bind: ["status" => status, "error" => error, "chunk_count" => n, "steps" => steps, "updated_at" => now],
            where: ("id", id),
            tenant: Some(tenant_id)
        )?;
    } else {
        raisfast_derive::crud_update!(
            pool,
            "kb_documents",
            bind: ["status" => status, "error" => error, "steps" => steps, "updated_at" => now],
            where: ("id", id),
            tenant: Some(tenant_id)
        )?;
    }
    Ok(())
}

pub async fn delete_document(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "kb_documents",
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

/// All documents of one KB (storage cleanup input for KB delete).
pub async fn find_documents_by_kb(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Vec<KbDocument>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "kb_documents",
        KbDocument,
        where: ("kb_id", kb_id),
        tenant: Some(tenant_id)
    )?)
}

/// Refresh the denormalized chunk counter after chunk-level edits/deletes.
pub async fn set_chunk_count(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    n: i64,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_documents",
        bind: ["chunk_count" => n, "updated_at" => now],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

pub async fn delete_documents_by_kb(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "kb_documents",
        where: ("kb_id", kb_id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

/// Batch resolve document titles (chunks list column). Ids missing from
/// the table are silently absent from the map.
pub async fn find_doc_titles_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
) -> AppResult<std::collections::HashMap<i64, String>> {
    let mut out = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(Driver::ph).collect();
    let sql = format!(
        "SELECT id, title FROM kb_documents WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, (i64, String)>(crate::db::safe_sql(&sql));
    for id in ids {
        query = query.bind(id);
    }
    for (id, title) in query.fetch_all(pool).await.map_err(|e| {
        AppError::Internal(anyhow::anyhow!(e.to_string()))
    })? {
        out.insert(id, title);
    }
    Ok(out)
}

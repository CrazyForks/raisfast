//! `kb_documents` table — document row model, ingest status machine
//! (`pending → parsing → chunking → embedding → ready | failed`,
//! [抄WK:types/knowledge_process.go]) and queries.

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

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
    /// `pending` | `parsing` | `chunking` | `embedding` | `ready` | `failed`
    /// [抄WK:types/knowledge_process.go].
    pub status: String,
    pub error: Option<String>,
    pub chunk_count: i64,
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

pub async fn set_document_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: &str,
    error: Option<&str>,
    chunk_count: Option<i64>,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    if let Some(n) = chunk_count {
        raisfast_derive::crud_update!(
            pool,
            "kb_documents",
            bind: ["status" => status, "error" => error, "chunk_count" => n, "updated_at" => now],
            where: ("id", id),
            tenant: Some(tenant_id)
        )?;
    } else {
        raisfast_derive::crud_update!(
            pool,
            "kb_documents",
            bind: ["status" => status, "error" => error, "updated_at" => now],
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

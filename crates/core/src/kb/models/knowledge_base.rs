//! `kb_knowledge_bases` table — KB row model and queries
//! (kb-technical-design §2, decision D4).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbKnowledgeBase {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub name: String,
    /// What this KB is for (human note, shown in admin).
    pub description: Option<String>,
    pub slug: String,
    /// `document` | `faq` (wiki is an indexing-strategy flag on document
    /// KBs, decision D4 [抄WK:knowledgebase.go IsWikiEnabled]).
    pub kind: String,
    pub indexing_strategy: Option<Value>,
    pub chunking_config: Option<Value>,
    pub embedding_model: Option<String>,
    pub embedding_dim: Option<i64>,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Create-KB command (repo `commands::*Cmd` pattern [抄RF]).
#[derive(Debug, Clone)]
pub struct CreateKbCmd {
    pub name: String,
    /// What this KB is for (human note, shown in admin).
    pub description: Option<String>,
    pub slug: String,
    pub kind: String,
    pub indexing_strategy: Option<Value>,
    pub embedding_model: Option<String>,
    pub embedding_dim: Option<i64>,
}

pub async fn create_kb(
    pool: &crate::db::Pool,
    cmd: &CreateKbCmd,
    tenant_id: &str,
) -> AppResult<KbKnowledgeBase> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_knowledge_bases",
        [
            "id" => id,
            "name" => cmd.name.as_str(),
            "description" => cmd.description.as_deref(),
            "slug" => cmd.slug.as_str(),
            "kind" => cmd.kind.as_str(),
            "indexing_strategy" => cmd.indexing_strategy.clone(),
            "embedding_model" => cmd.embedding_model.as_deref(),
            "embedding_dim" => cmd.embedding_dim,
            "created_at" => now
        ],
        tenant: Some(tenant_id)
    )?;
    find_kb_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("kb insert returned no row")))
}

/// Update-KB command — only mutable metadata (name/slug/description/status);
/// kind and embedding config are immutable after creation (WeKnora
/// `vector_store_id` precedent: vectors are pinned to model+dim).
#[derive(Debug, Clone)]
pub struct UpdateKbCmd {
    pub name: String,
    pub description: Option<String>,
    pub slug: String,
    pub status: String,
}

pub async fn update_kb(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    cmd: &UpdateKbCmd,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_knowledge_bases",
        bind: [
            "name" => cmd.name.as_str(),
            "description" => cmd.description.as_deref(),
            "slug" => cmd.slug.as_str(),
            "status" => cmd.status.as_str(),
            "updated_at" => now
        ],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

pub async fn find_kb_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Option<KbKnowledgeBase>> {
    Ok(raisfast_derive::crud_find!(
        pool,
        "kb_knowledge_bases",
        KbKnowledgeBase,
        where: ("id", id),
        tenant: Some(tenant_id)
    )?)
}

pub async fn delete_kb(pool: &crate::db::Pool, id: SnowflakeId, tenant_id: &str) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "kb_knowledge_bases",
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

pub async fn list_kbs(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: &str,
) -> AppResult<(Vec<KbKnowledgeBase>, i64)> {
    Ok(raisfast_derive::crud_query_paged!(
        pool,
        KbKnowledgeBase,
        table: "kb_knowledge_bases",
        order_by: "created_at DESC",
        tenant: Some(tenant_id),
        page: page,
        page_size: page_size
    ))
}

/// Batch resolve KB names (chunks list column). Ids missing from the
/// table are silently absent from the map.
pub async fn find_kb_names_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
) -> AppResult<std::collections::HashMap<i64, String>> {
    let mut out = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(Driver::ph).collect();
    let sql = format!(
        "SELECT id, name FROM kb_knowledge_bases WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, (i64, String)>(crate::db::safe_sql(&sql));
    for id in ids {
        query = query.bind(id);
    }
    for (id, name) in query
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?
    {
        out.insert(id, name);
    }
    Ok(out)
}

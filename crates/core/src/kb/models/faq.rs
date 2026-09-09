//! `kb_faqs` table — standard Q→A pairs (kb-technical-design §8).
//!
//! Rows are human-curated (M5 adds CRUD + sedimentation); the retrieval
//! pipeline reads them for FAQ content injection (S7①, [抄WK:merge_faq.go]).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbFaq {
    pub id: SnowflakeId,
    pub kb_id: SnowflakeId,
    pub tenant_id: String,
    pub standard_question: String,
    pub similar_questions: Option<Value>,
    /// Multiple accepted answers; S7① renders the first (or all) into the
    /// context [抄WK:types/faq.go 多答案].
    pub answers: Value,
    pub tags: Option<Value>,
    pub enabled: bool,
    pub hit_count: i64,
    pub created_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl KbFaq {
    /// Render the standard Q/A block injected into the generation context
    /// [抄WK:merge_faq.go buildFAQAnswerContent 语义].
    pub fn render_for_context(&self) -> String {
        let answers: Vec<String> = self
            .answers
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut out = format!("Q: {}\n", self.standard_question);
        if !answers.is_empty() {
            out.push_str("Answer:\n");
            for ans in answers {
                out.push_str(&format!("- {ans}\n"));
            }
        }
        out.trim_end().to_string()
    }
}

/// Fetch enabled FAQs by ids (candidate content injection).
pub async fn find_faqs_by_ids(pool: &crate::db::Pool, ids: &[i64]) -> AppResult<Vec<KbFaq>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(Driver::ph).collect();
    let sql = format!(
        "SELECT * FROM kb_faqs WHERE enabled = TRUE AND id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, KbFaq>(crate::db::safe_sql(&sql));
    for id in ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await.map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
    })?;
    Ok(rows)
}

/// Create-FAQ command.
#[derive(Debug, Clone)]
pub struct CreateFaqCmd {
    pub kb_id: SnowflakeId,
    pub standard_question: String,
    pub similar_questions: Vec<String>,
    pub answers: Vec<String>,
    /// Created disabled when drafted from query logs (P1 human gate).
    pub enabled: bool,
    pub created_by: Option<SnowflakeId>,
}

pub async fn create_faq(
    pool: &crate::db::Pool,
    cmd: &CreateFaqCmd,
    tenant_id: &str,
) -> AppResult<KbFaq> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_faqs",
        [
            "id" => id,
            "kb_id" => cmd.kb_id,
            "tenant_id" => tenant_id,
            "standard_question" => cmd.standard_question.as_str(),
            "similar_questions" => serde_json::json!(cmd.similar_questions),
            "answers" => serde_json::json!(cmd.answers),
            "enabled" => cmd.enabled,
            "created_by" => cmd.created_by,
            "created_at" => now
        ],
        tenant: Some(tenant_id)
    )?;
    Ok(KbFaq {
        id,
        kb_id: cmd.kb_id,
        tenant_id: tenant_id.to_string(),
        standard_question: cmd.standard_question.clone(),
        similar_questions: Some(serde_json::json!(cmd.similar_questions)),
        answers: serde_json::json!(cmd.answers),
        tags: None,
        enabled: cmd.enabled,
        hit_count: 0,
        created_by: cmd.created_by,
        created_at: now,
        updated_at: now,
    })
}

pub async fn set_faq_enabled(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    enabled: bool,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_faqs",
        bind: ["enabled" => enabled, "updated_at" => now],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

pub async fn delete_faq(pool: &crate::db::Pool, id: SnowflakeId, tenant_id: &str) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "kb_faqs", where: ("id", id), tenant: Some(tenant_id))?;
    Ok(())
}

pub async fn delete_faqs_by_kb(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "kb_faqs", where: ("kb_id", kb_id), tenant: Some(tenant_id))?;
    Ok(())
}

pub async fn bump_hit_count(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    let sql = format!(
        "UPDATE kb_faqs SET hit_count = hit_count + 1 WHERE id = {}",
        Driver::ph(1)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(i64::from(id))
        .execute(pool)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
        })?;
    Ok(())
}

/// Question variants to embed for retrieval (standard + similar).
pub fn question_variants(faq: &KbFaq) -> Vec<String> {
    let mut variants = vec![faq.standard_question.clone()];
    if let Some(similar) = faq.similar_questions.as_ref().and_then(|s| s.as_array()) {
        variants.extend(
            similar
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string)),
        );
    }
    variants
}

pub async fn list_faqs(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
    page: i64,
    page_size: i64,
    tenant_id: &str,
) -> AppResult<(Vec<KbFaq>, i64)> {
    Ok(raisfast_derive::crud_query_paged!(
        pool,
        KbFaq,
        table: "kb_faqs",
        where: ("kb_id", kb_id),
        order_by: "created_at DESC",
        tenant: Some(tenant_id),
        page: page,
        page_size: page_size
    ))
}

/// Update-FAQ command (retrieval re-index handled by the service layer).
#[derive(Debug, Clone)]
pub struct UpdateFaqCmd {
    pub standard_question: String,
    pub similar_questions: Vec<String>,
    pub answers: Vec<String>,
}

pub async fn update_faq(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    cmd: &UpdateFaqCmd,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_faqs",
        bind: [
            "standard_question" => cmd.standard_question.as_str(),
            "similar_questions" => serde_json::json!(cmd.similar_questions),
            "answers" => serde_json::json!(cmd.answers),
            "updated_at" => now
        ],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

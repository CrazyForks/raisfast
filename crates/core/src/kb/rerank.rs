//! S5 rerank seam — optional relevance reranking over the llm 底座
//! (kb-technical-design §6.1, revised 2026-09-17).
//!
//! Routing/auth/failover/billing all live in the LLM gateway (`/v1/rerank`
//! relay + `LlmModelType::Rerank` channels, §10.2); the KB side only picks
//! the model name via `RAISFAST_KB_RERANK_MODEL`. Empty model = no reranker
//! wired → S5 passthrough (v1 behavior).

use crate::errors::app_error::{AppError, AppResult};

/// One rerank input unit (§6.1.1). `unit_id` is the `kb_chunks.id` snowflake;
/// `text` is the same breadcrumb+content payload the S2 FTS index sees.
#[derive(Debug, Clone)]
pub struct RerankDoc {
    pub unit_id: i64,
    pub text: String,
}

/// Rerank seam: production wraps the llm 底座 facade (§10.2 唯一入口,
/// mirrors [`crate::kb::service::KbEmbedder::embed_for`]'s per-KB model
/// param); tests inject stubs.
#[async_trait::async_trait]
pub trait KbReranker: Send + Sync {
    /// Relevance scores (0..1, higher = more relevant), equal length and
    /// same order as `docs` — call sites never remap indexes. `model` is
    /// the per-KB resolved rerank model (KB row → global default).
    async fn rerank(
        &self,
        tenant: &str,
        model: &str,
        query: &str,
        docs: &[RerankDoc],
    ) -> AppResult<Vec<f32>>;
}

// Docs per `/rerank` request — §6.1.4 分批：候选超批大小时切片串行.
// Global (transport concern), mirrors `embed_batch_size`.

/// Retry shape [抄RF:ProviderEmbedder::embed_batched（同源抄WK
/// batchEmbedWithBackoff）]: 5 attempts, backoff 200ms → 3.2s.
const RERANK_RETRY_ATTEMPTS: usize = 5;
const RERANK_RETRY_BASE_DELAY_MS: u64 = 200;

/// Production reranker over the llm 底座 facade — the ONLY entry
/// (model resolution/routing/failover/billing in the gateway, §10.2).
pub struct ProviderReranker {
    router: std::sync::Arc<crate::llm::service::LlmRouter>,
    /// Docs per `/rerank` request (global, like `embed_batch_size`).
    batch_size: usize,
}

impl ProviderReranker {
    pub fn new(router: std::sync::Arc<crate::llm::service::LlmRouter>, batch_size: usize) -> Self {
        Self { router, batch_size }
    }

    /// Slice docs into fixed-size batches and rerank them sequentially,
    /// retrying each batch with exponential backoff (§6.1.4 分批).
    async fn rerank_batched(
        &self,
        tenant: &str,
        model: &str,
        query: &str,
        docs: &[RerankDoc],
    ) -> AppResult<Vec<f32>> {
        let mut out = Vec::with_capacity(docs.len());
        for batch in docs.chunks(self.batch_size.max(1)) {
            let mut delay = RERANK_RETRY_BASE_DELAY_MS;
            let mut last_err: Option<String> = None;
            let mut scores: Option<Vec<f32>> = None;
            for attempt in 0..RERANK_RETRY_ATTEMPTS {
                match self.rerank_once(tenant, model, query, batch).await {
                    Ok(v) if v.len() == batch.len() => {
                        scores = Some(v);
                        break;
                    }
                    Ok(v) => {
                        last_err = Some(format!(
                            "rerank score count mismatch: batch {} got {}",
                            batch.len(),
                            v.len()
                        ));
                    }
                    Err(e) => last_err = Some(e),
                }
                tracing::warn!(
                    "rerank batch ({}/{} docs) attempt {}/{} failed: {}",
                    batch.len(),
                    docs.len(),
                    attempt + 1,
                    RERANK_RETRY_ATTEMPTS,
                    last_err.as_deref().unwrap_or_default()
                );
                if attempt + 1 < RERANK_RETRY_ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay *= 2;
                }
            }
            match scores {
                Some(v) => out.extend(v),
                None => {
                    return Err(AppError::ServiceUnavailable(format!(
                        "rerank: {}",
                        last_err.unwrap_or_else(|| "unknown error".into())
                    )));
                }
            }
        }
        Ok(out)
    }

    /// 单批重排：唯一入口 llm 底座（路由/failover/计费/日志，§10.2）。
    async fn rerank_once(
        &self,
        tenant: &str,
        model: &str,
        query: &str,
        batch: &[RerankDoc],
    ) -> Result<Vec<f32>, String> {
        let texts: Vec<&str> = batch.iter().map(|d| d.text.as_str()).collect();
        let results = self
            .router
            .call(tenant, crate::llm::models::log::LogSource::Kb)
            .rerank(model, query, &texts)
            .await
            .map_err(|e| e.to_string())?;
        Ok(backfill_scores(batch.len(), &results))
    }
}

#[async_trait::async_trait]
impl KbReranker for ProviderReranker {
    async fn rerank(
        &self,
        tenant: &str,
        model: &str,
        query: &str,
        docs: &[RerankDoc],
    ) -> AppResult<Vec<f32>> {
        self.rerank_batched(tenant, model, query, docs).await
    }
}

/// Backfill best-first `RerankResult{index, score}` rows into an
/// equal-length, same-order score vector — docs the model did not score
/// default to 0.0 (unranked = irrelevant; §6.1.1 等长同序契约).
fn backfill_scores(len: usize, results: &[raisfast_agent::provider::RerankResult]) -> Vec<f32> {
    let mut scores = vec![0.0_f32; len];
    for r in results {
        if r.index < len {
            scores[r.index] = r.relevance_score;
        }
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(index: usize, score: f32) -> raisfast_agent::provider::RerankResult {
        raisfast_agent::provider::RerankResult {
            index,
            relevance_score: score,
        }
    }

    #[test]
    fn backfill_maps_by_index_best_first() {
        // Best-first rows land on their input positions, not their rank.
        let scores = backfill_scores(3, &[result(2, 0.9), result(0, 0.5), result(1, 0.7)]);
        assert_eq!(scores, vec![0.5, 0.7, 0.9]);
    }

    #[test]
    fn backfill_defaults_unscored_to_zero() {
        let scores = backfill_scores(3, &[result(0, 0.8)]);
        assert_eq!(scores, vec![0.8, 0.0, 0.0]);
    }

    #[test]
    fn backfill_ignores_out_of_range_index() {
        let scores = backfill_scores(2, &[result(0, 0.4), result(9, 1.0)]);
        assert_eq!(scores, vec![0.4, 0.0]);
    }
}

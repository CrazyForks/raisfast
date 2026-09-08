//! S2 recall — parallel BM25 + dense retrieval
//! [抄WK:chat_pipeline/search_parallel.go 语义].

use crate::errors::app_error::AppResult;
use crate::kb::pipeline::understand::UnderstoodQuery;
use crate::kb::service::KbDeps;

/// One path's raw hits: (unit_id, path-local score).
#[derive(Debug, Clone)]
pub struct RawHits {
    pub bm25: Vec<(i64, f32)>,
    pub dense: Vec<(i64, f32)>,
}

/// Alias kept for the module re-export contract.
pub type RecallOutcome = RawHits;

/// Recall from both paths in parallel for one KB. Query embedding uses the
/// same embedder as ingestion (dimension consistency by construction).
pub async fn recall(deps: &KbDeps, kb_id: i64, query: &UnderstoodQuery) -> AppResult<RawHits> {
    let top_k = deps.config.kb.top_k as usize;

    let bm25_future = {
        let kbsearch = deps.kbsearch.clone();
        let text = query.text.clone();
        async move { kbsearch.search(kb_id, &text, top_k).await }
    };
    let dense_future = async {
        let texts = [query.text.as_str()];
        let vectors = deps.embedder.embed(&texts).await?;
        let Some(embedding) = vectors.first() else {
            return Ok(Vec::new());
        };
        deps.vector
            .search(kb_id, embedding, top_k, None)
            .await
            .map(|hits| hits.into_iter().map(|h| (h.unit_id, h.score)).collect())
    };

    let (bm25, dense) = tokio::join!(bm25_future, dense_future);
    Ok(RawHits {
        bm25: bm25?.into_iter().map(|h| (h.unit_id, h.score)).collect(),
        dense: dense?,
    })
}

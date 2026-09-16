//! S5 rerank — optional relevance rerank over the llm 底座
//! [抄WK:chat_pipeline/rerank.go 管道位；修订 2026-09-17：模型经 llm 底座
//! 路由（§10.2），KB 侧只选模型名，见 kb-technical-design §6.1].
//!
//! Semantics (§6.1.4): the rerank score REPLACES the fused RRF score for
//! ordering, threshold filtering and the S11 fallback verdict; wiki boost
//! (S6) then multiplies the rerank score. Failures degrade to the RRF
//! order — rerank is a gain, never a dependency [抄WK:rerank.go 容错：
//! 错误记日志不中断管道].

use serde_json::json;

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::kb::rerank::RerankDoc;
use crate::kb::service::KbDeps;
use crate::kb::trace::RunRecorder;

use super::Candidate;

/// Per-query S5 rerank config, resolved from the KB scope (§6.1.2):
/// the first KB with a `rerank_model` provides the whole trio (single
/// model per merged pool, WK RetrievalConfig precedent); null fields on
/// that row fall back to the global env defaults.
#[derive(Debug, Clone)]
pub(super) struct ResolvedRerank {
    pub model: String,
    pub window: u32,
    pub threshold: f32,
}

/// Raw rerank columns off a KB row (resolution query shape).
#[derive(sqlx::FromRow)]
struct KbRerankRow {
    id: i64,
    #[sqlx(rename = "rerank_model")]
    model: Option<String>,
    #[sqlx(rename = "rerank_window")]
    window: Option<i64>,
    #[sqlx(rename = "rerank_threshold")]
    threshold: Option<f64>,
}

/// Resolve S5 rerank for a KB scope: first KB row (in scope order) with a
/// non-empty `rerank_model` wins; without any per-KB override the global
/// `RAISFAST_KB_RERANK_MODEL` default applies; neither → `None`
/// (passthrough). Rerank trio is freely editable per KB — query-time
/// behavior, nothing baked (unlike the pinned embedding config).
pub(super) async fn resolve(deps: &KbDeps, kbs: &[i64]) -> AppResult<Option<ResolvedRerank>> {
    let global = || {
        deps.config
            .kb
            .rerank_model
            .as_deref()
            .filter(|m| !m.is_empty())
    };
    if deps.reranker.is_none() {
        return Ok(None);
    }
    let global_default = || ResolvedRerank {
        model: global().unwrap_or_default().to_string(),
        window: deps.config.kb.rerank_window,
        threshold: deps.config.kb.rerank_threshold,
    };
    if kbs.is_empty() {
        return Ok(global().map(|_| global_default()));
    }
    let placeholders: Vec<String> = (1..=kbs.len()).map(crate::db::Driver::ph).collect();
    let sql = format!(
        "SELECT id, rerank_model, rerank_window, rerank_threshold \
         FROM kb_knowledge_bases WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, KbRerankRow>(crate::db::safe_sql(&sql));
    for id in kbs {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(&deps.pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    let by_id: std::collections::HashMap<i64, KbRerankRow> =
        rows.into_iter().map(|r| (r.id, r)).collect();
    for kb_id in kbs {
        let Some(row) = by_id.get(kb_id) else {
            continue;
        };
        if row.model.as_deref().is_some_and(|m| !m.is_empty()) {
            return Ok(Some(ResolvedRerank {
                model: row.model.clone().unwrap_or_default(),
                window: row
                    .window
                    .map(|w| w.max(1) as u32)
                    .unwrap_or(deps.config.kb.rerank_window),
                threshold: row
                    .threshold
                    .map(|t| t as f32)
                    .unwrap_or(deps.config.kb.rerank_threshold),
            }));
        }
    }
    Ok(global().map(|_| global_default()))
}

/// 送排文本 = `breadcrumb + "\n" + content`，与 S2 双路召回所索引的 FTS
/// 文本同构（重排器看到的与检索器看到的是同一份文本）[§6.1.1].
fn rerank_text(chunk: &crate::kb::models::chunk::KbChunk) -> String {
    match &chunk.breadcrumb {
        Some(b) if !b.is_empty() => format!("{b}\n{}", chunk.content),
        _ => chunk.content.clone(),
    }
}

/// Bounded top-3 list for the s5 summary (§6.1.4 最小埋点：S5 前后 top-3
/// 变化即 rerank 开关效果).
fn top3(candidates: &[Candidate]) -> Vec<serde_json::Value> {
    candidates
        .iter()
        .take(3)
        .map(|c| {
            json!({
                "id": c.unit_id,
                "score": (c.score * 10000.0).round() / 10000.0,
                "title": super::chunk_title(&c.chunk),
            })
        })
        .collect()
}

/// Degrade without failing the query: keep the incoming (RRF) order and
/// mark the stage degraded (run-level degraded flag via the recorder).
fn degrade(trace: &mut RunRecorder, reason: String, before_top3: Vec<serde_json::Value>) {
    tracing::warn!("[kb] rerank degraded to RRF order: {reason}");
    trace.end_stage(
        crate::kb::trace::STAGE_DEGRADED,
        json!({ "fallback": "rrf", "error": reason, "before_top3": before_top3 }),
        None,
    );
}

/// Stage body: rerank `hydrated` in place when a rerank config resolves,
/// then threshold-filter and cut back to `top_k`. Records
/// skipped/ok/degraded into `trace`; never fails the query. §6.1.4 阈值
/// 过滤后不足 top_k 不补召回（宁可少上下文，不引入重排器已判低分的噪声）.
pub(super) async fn rerank_stage(
    deps: &KbDeps,
    tenant: &str,
    rerank: Option<&ResolvedRerank>,
    query: &str,
    hydrated: &mut Vec<Candidate>,
    trace: &mut RunRecorder,
) {
    let Some(reranker) = deps.reranker.clone() else {
        trace.end_stage(
            crate::kb::trace::STAGE_SKIPPED,
            json!({ "reason": "no_reranker_wired" }),
            None,
        );
        return;
    };
    let Some(cfg) = rerank else {
        trace.end_stage(
            crate::kb::trace::STAGE_SKIPPED,
            json!({ "reason": "no_rerank_model" }),
            None,
        );
        return;
    };
    if hydrated.is_empty() {
        // 抄WK:rerank.go empty_search_result skip.
        trace.end_stage(
            crate::kb::trace::STAGE_SKIPPED,
            json!({ "reason": "empty_candidates", "model": cfg.model }),
            None,
        );
        return;
    }
    let before_top3 = top3(hydrated);
    let docs: Vec<RerankDoc> = hydrated
        .iter()
        .map(|c| RerankDoc {
            unit_id: c.unit_id,
            text: rerank_text(&c.chunk),
        })
        .collect();
    match reranker.rerank(tenant, &cfg.model, query, &docs).await {
        Ok(scores) if scores.len() == hydrated.len() => {
            // 分数替换：重排分替换融合分参与排序与 S11 阈值判断.
            for (c, s) in hydrated.iter_mut().zip(scores) {
                c.score = s;
            }
            let before_filter = hydrated.len();
            hydrated.retain(|c| c.score >= cfg.threshold);
            let dropped_by_threshold = before_filter - hydrated.len();
            hydrated.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            hydrated.truncate(deps.config.kb.top_k as usize);
            trace.end_stage(
                crate::kb::trace::STAGE_OK,
                json!({
                    "model": cfg.model,
                    "threshold": cfg.threshold,
                    "kept": hydrated.len(),
                    "dropped_by_threshold": dropped_by_threshold,
                    "before_top3": before_top3,
                    "after_top3": top3(hydrated),
                }),
                None,
            );
        }
        Ok(scores) => {
            degrade(
                trace,
                format!(
                    "rerank score count mismatch: {} docs, {} scores",
                    hydrated.len(),
                    scores.len()
                ),
                before_top3,
            );
        }
        Err(e) => degrade(trace, e.to_string(), before_top3),
    }
}

//! S6 wiki boost + S7 merge — FAQ injection, parent expansion, dedup
//! [抄WK:wiki_boost.go + merge_faq.go + merge_expand.go + merge_overlap.go].

use std::collections::HashMap;
use std::sync::Arc;

use crate::errors::app_error::AppResult;
use crate::kb::models::chunk::KbChunk;
use crate::kb::pipeline::Candidate;
use crate::kb::pipeline::ContextUnit;
use crate::kb::service::KbDeps;

/// S6: multiply `wiki_page` unit scores and re-sort stably
/// [抄WK:wiki_boost.go——常量来自配置，默认 1.3 同值].
pub fn apply_wiki_boost(candidates: &mut [Candidate], boost: f32) {
    let mut boosted = 0;
    for c in candidates.iter_mut() {
        if c.chunk.kind == "wiki_page" {
            c.score *= boost;
            boosted += 1;
        }
    }
    if boosted > 0 {
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}

/// S7: merge candidates into context units.
///
/// ① FAQ units: content replaced by the standard Q/A block, flagged to pin
///    at the top of the assembled context [抄WK:merge_faq.go].
/// ② Document child units: expanded to their parent chunk's content
///    [抄WK:merge_expand.go + parent-child retrieval].
/// ③ Units whose rendered content matches an already-kept unit are dropped
///    [抄WK:merge_overlap.go 去重语义].
pub async fn merge_units(deps: &KbDeps, candidates: &[Candidate]) -> AppResult<Vec<ContextUnit>> {
    // ① FAQ hydration
    let faq_ids: Vec<i64> = candidates
        .iter()
        .filter(|c| c.chunk.kind == "faq")
        .map(|c| c.chunk.faq_id.map(i64::from).unwrap_or(c.unit_id))
        .collect();
    let faqs: HashMap<i64, crate::kb::models::faq::KbFaq> = if faq_ids.is_empty() {
        HashMap::new()
    } else {
        crate::kb::models::faq::find_faqs_by_ids(&deps.pool, &faq_ids)
            .await?
            .into_iter()
            .map(|f| (i64::from(f.id), f))
            .collect()
    };

    // ② Parent expansion map: child unit_id → parent chunk (bulk fetch).
    let parent_ids: Vec<i64> = candidates
        .iter()
        .filter(|c| c.chunk.kind == "document" && c.chunk.parent_id.is_some())
        .map(|c| c.chunk.parent_id.map(i64::from).unwrap_or_default())
        .collect();
    let parents: HashMap<i64, Arc<KbChunk>> = if parent_ids.is_empty() {
        HashMap::new()
    } else {
        crate::kb::models::chunk::find_chunks_by_ids(&deps.pool, &parent_ids)
            .await?
            .into_iter()
            .map(|c| (i64::from(c.id), Arc::new(c)))
            .collect()
    };

    let mut units: Vec<ContextUnit> = Vec::new();
    let mut seen_content: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in candidates {
        let unit = match c.chunk.kind.as_str() {
            "faq" => {
                let Some(faq) = faqs.get(&c.chunk.faq_id.map(i64::from).unwrap_or(-1)) else {
                    continue; // disabled or deleted FAQ — skip
                };
                ContextUnit {
                    unit_id: c.unit_id,
                    kind: "faq".into(),
                    title: faq.standard_question.clone(),
                    content: faq.render_for_context(),
                    score: c.score,
                    is_faq: true,
                }
            }
            "document" => {
                let source = match c.chunk.parent_id {
                    Some(pid) => parents.get(&i64::from(pid)).unwrap_or(&c.chunk),
                    None => &c.chunk,
                };
                ContextUnit {
                    unit_id: c.unit_id,
                    kind: "document".into(),
                    title: source
                        .breadcrumb
                        .clone()
                        .unwrap_or_else(|| "文档片段".into()),
                    content: source.content.clone(),
                    score: c.score,
                    is_faq: false,
                }
            }
            // wiki_page (M4) falls through to plain content until distillation lands.
            other => ContextUnit {
                unit_id: c.unit_id,
                kind: other.to_string(),
                title: c
                    .chunk
                    .breadcrumb
                    .clone()
                    .unwrap_or_else(|| "知识页".into()),
                content: c.chunk.content.clone(),
                score: c.score,
                is_faq: false,
            },
        };
        // ③ dedup on rendered content (post-expansion collisions collapse).
        if seen_content.insert(unit.content.clone()) {
            units.push(unit);
        }
    }
    Ok(units)
}

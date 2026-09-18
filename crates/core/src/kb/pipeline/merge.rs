//! S6 wiki boost + S7 merge — FAQ injection, parent expansion, dedup,
//! same-document adjacent-unit stitching
//! [抄WK:wiki_boost.go + merge_faq.go + merge_expand.go + merge_overlap.go].
//! Stitching is a strategy port of WK `mergeSequentialChunks` with declared
//! deltas (see `stitch_adjacent`): our units are parent-expanded before the
//! merge and parent spans tile the source markdown disjointly, so WK's
//! ChunkIndex/coordinate trusted-pair classification collapses to a byte-gap
//! test on (doc_id, byte_start/end).

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

/// S7 outcome bookkeeping, surfaced in the s7 trace stage.
#[derive(Debug, Default, Clone, Copy)]
pub struct MergeStats {
    /// Units dropped by rendered-content dedup.
    pub dedup_dropped: usize,
    /// Adjacent-unit groups stitched into one unit each.
    pub stitched_groups: usize,
    /// Units absorbed into stitched groups.
    pub stitched_units: usize,
    /// Neighbor parents pulled into short units by ⑤.
    pub neighbors_added: usize,
}

/// Byte-gap tolerance for stitching. Parent spans exactly tile the source
/// markdown (text-splitter yields contiguous slices); the only gaps come
/// from anchor-only parents dropped at ingest (a blanked page-mark line,
/// ~20 bytes). Anything wider is a genuinely separate passage [自造常数——
/// WK 无对应值，按页锚点行长度级取保守值].
const STITCH_GAP_TOLERANCE: i64 = 128;

/// Short-context expansion thresholds [抄WK:merge_expand.go minLen=350 /
/// maxLen=850，同值]. A document unit whose content is under `MIN` chars
/// pulls its adjacent parents (⑤) until the window reaches `MIN`; the
/// window never exceeds `MAX` chars.
const NEIGHBOR_MIN_CHARS: usize = 350;
const NEIGHBOR_MAX_CHARS: usize = 850;

/// S7: merge candidates into context units.
///
/// ① FAQ units: content replaced by the standard Q/A block, flagged to pin
///    at the top of the assembled context [抄WK:merge_faq.go].
/// ② Document child units: expanded to their parent chunk's content
///    [抄WK:merge_expand.go + parent-child retrieval].
/// ③ Units whose rendered content matches an already-kept unit are dropped
///    [抄WK:merge_overlap.go 去重语义].
/// ④ Document units from the same document whose parent spans are adjacent
///    are stitched into one unit [抄WK:merge_overlap.go mergeSequentialChunks，
///    see `stitch_adjacent` for the declared deltas] — answers spanning
///    several pages/parents then reach the LLM as one continuous passage.
pub async fn merge_units(
    deps: &KbDeps,
    candidates: &[Candidate],
) -> AppResult<(Vec<ContextUnit>, MergeStats)> {
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
    // unit_id → (doc_id, byte_start, byte_end) of the expanded parent span,
    // kept alongside so ④ can stitch without widening `ContextUnit`.
    let mut coords: HashMap<i64, (i64, i64, i64)> = HashMap::new();
    // unit_id → source parent seq, for the ⑤ neighbor walk.
    let mut seqs: HashMap<i64, i64> = HashMap::new();
    let mut seen_content: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stats = MergeStats::default();
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
                if let Some(doc) = source.doc_id {
                    coords.insert(
                        c.unit_id,
                        (i64::from(doc), source.byte_start, source.byte_end),
                    );
                    seqs.insert(c.unit_id, source.seq);
                }
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
        } else {
            stats.dedup_dropped += 1;
        }
    }
    let mut present: std::collections::HashSet<(i64, i64)> =
        coords.values().map(|(d, s, _)| (*d, *s)).collect();
    stats.neighbors_added =
        expand_short_units(deps, &mut units, &coords, &seqs, &mut present).await?;
    let (units, stitched_groups, stitched_units) = stitch_adjacent(units, &coords);
    stats.stitched_groups = stitched_groups;
    stats.stitched_units = stitched_units;
    Ok((units, stats))
}

/// ⑤ Short-context neighbor expansion [抄WK:merge_expand.go
/// `expandShortContextWithNeighbors` — strategy port with declared deltas].
/// A document unit under `NEIGHBOR_MIN_CHARS` grows its window by walking
/// adjacent parents alternately (prev, next, prev…) until it reaches `MIN`
/// chars; the window never exceeds `MAX`. Declared deltas vs WK:
/// - the neighbor chain is a `(doc_id, seq)` walk [自造 field mapping — we
///   have no PreChunkID/NextChunkID columns], resolved per step by
///   `find_parent_neighbors`;
/// - truncation keeps the side NEAREST the base (tail of prev / head of
///   next); WK front-truncates the merged string, which is safe for its
///   ~1K flat chunks but would discard the base entirely for our parents
///   (up to 4096 chars);
/// - neighbors already present in the context (recalled — including
///   stitch-absorbed — units) are skipped, walking past them outward
///   [自造 — our stitch/dedup runs in this same pass; WK dedups elsewhere];
/// - junctions always join with `\n\n` [抄WK:JoinChunkContent 分隔符].
///
/// Returns the number of neighbors pulled in.
async fn expand_short_units(
    deps: &KbDeps,
    units: &mut [ContextUnit],
    coords: &HashMap<i64, (i64, i64, i64)>,
    seqs: &HashMap<i64, i64>,
    present: &mut std::collections::HashSet<(i64, i64)>,
) -> AppResult<usize> {
    let mut added = 0usize;
    for u in units.iter_mut() {
        if u.kind != "document" || u.is_faq {
            continue;
        }
        let base_chars: Vec<char> = u.content.chars().collect();
        if base_chars.len() >= NEIGHBOR_MIN_CHARS {
            continue;
        }
        let Some(&(doc, _, _)) = coords.get(&u.unit_id) else {
            continue;
        };
        let Some(mut prev_cursor) = seqs.get(&u.unit_id).copied() else {
            continue;
        };
        let mut next_cursor = prev_cursor;
        let mut prev_acc = String::new();
        let mut next_acc = String::new();
        let mut total = base_chars.len();
        let mut prev_alive = true;
        let mut next_alive = true;
        while total < NEIGHBOR_MIN_CHARS && (prev_alive || next_alive) {
            let budget = NEIGHBOR_MAX_CHARS - total;
            if budget == 0 {
                break;
            }
            if prev_alive {
                match crate::kb::models::chunk::find_parent_neighbors(
                    &deps.pool,
                    crate::types::snowflake_id::SnowflakeId(doc),
                    prev_cursor,
                )
                .await?
                {
                    (Some(p), _) => {
                        let pid = i64::from(p.id);
                        if present.contains(&(doc, pid)) {
                            prev_cursor = p.seq; // already in context — walk past
                        } else {
                            let chars: Vec<char> = p.content.chars().collect();
                            let take = chars.len().min(budget);
                            let tail: String = chars[chars.len() - take..].iter().collect();
                            if prev_acc.is_empty() {
                                prev_acc = tail;
                            } else {
                                prev_acc = format!("{tail}\n\n{prev_acc}");
                            }
                            total += take;
                            added += 1;
                            present.insert((doc, pid));
                            prev_cursor = p.seq;
                            if take < chars.len() {
                                prev_alive = false; // budget exhausted on this side
                            }
                        }
                    }
                    (None, _) => prev_alive = false,
                }
            }
            if total >= NEIGHBOR_MIN_CHARS || !next_alive {
                continue;
            }
            let budget = NEIGHBOR_MAX_CHARS - total;
            if budget == 0 {
                break;
            }
            match crate::kb::models::chunk::find_parent_neighbors(
                &deps.pool,
                crate::types::snowflake_id::SnowflakeId(doc),
                next_cursor,
            )
            .await?
            {
                (_, Some(n)) => {
                    let nid = i64::from(n.id);
                    if present.contains(&(doc, nid)) {
                        next_cursor = n.seq; // already in context — walk past
                    } else {
                        let chars: Vec<char> = n.content.chars().collect();
                        let take = chars.len().min(budget);
                        let head: String = chars[..take].iter().collect();
                        if !next_acc.is_empty() {
                            next_acc.push_str("\n\n");
                        }
                        next_acc.push_str(&head);
                        total += take;
                        added += 1;
                        present.insert((doc, nid));
                        next_cursor = n.seq;
                        if take < chars.len() {
                            next_alive = false;
                        }
                    }
                }
                (_, None) => next_alive = false,
            }
        }
        let mut content = String::with_capacity(prev_acc.len() + u.content.len() + next_acc.len());
        if !prev_acc.is_empty() {
            content.push_str(&prev_acc);
            content.push_str("\n\n");
        }
        content.push_str(&u.content);
        if !next_acc.is_empty() {
            content.push_str("\n\n");
            content.push_str(&next_acc);
        }
        u.content = content;
    }
    Ok(added)
}

/// ④ Same-document adjacent-unit stitching [抄WK:chat_pipeline/merge_overlap.go
/// `mergeSequentialChunks` — strategy port with declared deltas]:
/// - WK classifies pre-expanded children by `ChunkIndex` + parser coordinates
///   (trusted pairs with an edited-content fallback, mergeExtend/mergeSubsume).
///   Our units are already parent-expanded and parent spans tile the source
///   markdown disjointly (text-splitter yields contiguous slices), so
///   subsumption is unreachable and classification reduces to a byte-gap test
///   on `(doc_id, byte_start/end)` [自造 field mapping — we have no ChunkIndex
///   column; byte spans carry the same ordering information].
/// - The merge runs post-expansion on parent content (WK runs pre-expansion);
///   grouping is per doc so unrelated documents never fuse.
/// - Groups must be detected in span order, so document units are sorted by
///   `(doc_id, byte_start)` first [照抄 WK: "Chunks MUST be pre-sorted by
///   ChunkIndex"].
///
/// The merged unit keeps the highest-scored member's identity and position
/// (units are score-ordered, so it surfaces where the best hit was); the
/// score is the members' max. Contents join in span order: gap 0 = direct
/// concat (the separator whitespace lives inside the parent contents),
/// gap > 0 (≤ `STITCH_GAP_TOLERANCE`) = `\n\n` (a dropped anchor-only parent
/// sat between them). Returns `(stitched_groups, stitched_units)`.
fn stitch_adjacent(
    units: Vec<ContextUnit>,
    coords: &HashMap<i64, (i64, i64, i64)>,
) -> (Vec<ContextUnit>, usize, usize) {
    // (unit index, doc_id, byte_start, byte_end), pre-sorted by span [照抄 WK
    // pre-sort]. Same-doc entries are then contiguous, so run detection only
    // ever compares against the immediately preceding group.
    let mut doc_units: Vec<(usize, i64, i64, i64)> = units
        .iter()
        .enumerate()
        .filter_map(|(i, u)| coords.get(&u.unit_id).map(|(d, s, e)| (i, *d, *s, *e)))
        .collect();
    doc_units.sort_by_key(|&(_, d, s, _)| (d, s));

    // Run detection: extend the open group while same doc and within the gap
    // tolerance; negative gaps (span overlap — unreachable by construction,
    // kept defensive) start a fresh group rather than risk duplicating text.
    let mut groups: Vec<(i64, Vec<usize>)> = Vec::new(); // (doc_id, unit idxs)
    let mut tails: Vec<i64> = Vec::new(); // group byte_end, parallel to groups
    for &(i, d, s, e) in &doc_units {
        let extendable = match (groups.last(), tails.last()) {
            (Some((gd, _)), Some(le)) => *gd == d && s >= *le && s - *le <= STITCH_GAP_TOLERANCE,
            _ => false,
        };
        if extendable {
            groups.last_mut().expect("checked above").1.push(i);
            *tails.last_mut().expect("checked above") = e;
        } else {
            groups.push((d, vec![i]));
            tails.push(e);
        }
    }

    // Build the merged replacement for each multi-member group and mark the
    // absorbed members. Head = highest-scoring member (positive-score pipeline;
    // `to_bits` preserves order on non-negative f32, ties broken by earlier
    // list position).
    let mut replacement: HashMap<usize, ContextUnit> = HashMap::new();
    let mut absorbed: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut stitched_groups = 0usize;
    let mut stitched_units = 0usize;
    for (_, idxs) in &groups {
        if idxs.len() < 2 {
            continue;
        }
        let head_idx = idxs
            .iter()
            .copied()
            .max_by_key(|&i| (units[i].score.to_bits(), std::cmp::Reverse(i)))
            .expect("group is non-empty");
        // Contents join in span order (group members are span-ordered): gap 0
        // = direct concat, small gap = blank-line separator.
        let mut content = String::new();
        let mut prev_end: Option<i64> = None;
        for &i in idxs {
            let (_, _, start, end) = doc_units
                .iter()
                .find(|(j, _, _, _)| *j == i)
                .copied()
                .expect("member index comes from doc_units");
            if let Some(pe) = prev_end
                && start - pe > 0
            {
                content.push_str("\n\n");
            }
            content.push_str(&units[i].content);
            prev_end = Some(end);
        }
        let head = &units[head_idx];
        replacement.insert(
            head_idx,
            ContextUnit {
                unit_id: head.unit_id,
                kind: head.kind.clone(),
                title: head.title.clone(),
                content,
                score: idxs.iter().map(|&i| units[i].score).fold(0.0_f32, f32::max),
                is_faq: false,
            },
        );
        for &i in idxs {
            if i != head_idx {
                absorbed.insert(i);
            }
        }
        stitched_groups += 1;
        stitched_units += idxs.len() - 1;
    }

    let out = units
        .into_iter()
        .enumerate()
        .filter_map(|(i, u)| {
            if absorbed.contains(&i) {
                None
            } else {
                Some(replacement.remove(&i).unwrap_or(u))
            }
        })
        .collect();
    (out, stitched_groups, stitched_units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: i64, content: &str, score: f32, kind: &str) -> ContextUnit {
        ContextUnit {
            unit_id: id,
            kind: kind.into(),
            title: format!("t{id}"),
            content: content.into(),
            score,
            is_faq: kind == "faq",
        }
    }

    fn coords_of(pairs: &[(i64, i64, i64, i64)]) -> HashMap<i64, (i64, i64, i64)> {
        pairs
            .iter()
            .map(|(id, d, s, e)| (*id, (*d, *s, *e)))
            .collect()
    }

    #[test]
    fn stitches_adjacent_spans_in_span_order_regardless_of_score_order() {
        // Page 3 scores higher and surfaces first; the stitch must still emit
        // page 2's text before page 3's.
        let units = vec![
            unit(3, "第三页内容", 0.9, "document"),
            unit(2, "第二页内容", 0.7, "document"),
        ];
        let coords = coords_of(&[(3, 1, 200, 300), (2, 1, 100, 200)]);
        let (out, groups, absorbed) = stitch_adjacent(units, &coords);
        assert_eq!((groups, absorbed), (1, 1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].unit_id, 3, "head = highest-scoring member");
        assert_eq!(out[0].score, 0.9);
        assert_eq!(out[0].content, "第二页内容第三页内容", "span order, gap 0");
    }

    #[test]
    fn small_gap_stitches_with_blank_line_separator() {
        let units = vec![
            unit(1, "前文", 0.9, "document"),
            unit(2, "后文", 0.5, "document"),
        ];
        // 20-byte gap = a dropped anchor-only parent between the two spans.
        let coords = coords_of(&[(1, 1, 0, 100), (2, 1, 120, 200)]);
        let (out, groups, absorbed) = stitch_adjacent(units, &coords);
        assert_eq!((groups, absorbed), (1, 1));
        assert_eq!(out[0].content, "前文\n\n后文");
    }

    #[test]
    fn wide_gap_keeps_units_separate() {
        let units = vec![
            unit(1, "甲", 0.9, "document"),
            unit(2, "乙", 0.5, "document"),
        ];
        let coords = coords_of(&[(1, 1, 0, 100), (2, 1, 5000, 5100)]);
        let (out, groups, absorbed) = stitch_adjacent(units, &coords);
        assert_eq!((groups, absorbed), (0, 0));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn different_documents_never_merge() {
        let units = vec![
            unit(1, "甲", 0.9, "document"),
            unit(2, "乙", 0.5, "document"),
        ];
        let coords = coords_of(&[(1, 1, 0, 100), (2, 2, 100, 200)]);
        let (out, groups, absorbed) = stitch_adjacent(units, &coords);
        assert_eq!((groups, absorbed), (0, 0));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn non_document_units_pass_through() {
        // FAQ (no coords) and a lone document unit stay untouched.
        let units = vec![
            unit(9, "问：A 答：B", 0.95, "faq"),
            unit(1, "唯一父块", 0.6, "document"),
        ];
        let coords = coords_of(&[(1, 1, 0, 100)]);
        let (out, groups, absorbed) = stitch_adjacent(units, &coords);
        assert_eq!((groups, absorbed), (0, 0));
        assert_eq!(out.len(), 2);
        assert!(out[0].is_faq);
    }

    #[test]
    fn chain_of_three_adjacent_parents_merges_into_one() {
        let units = vec![
            unit(2, "中", 0.8, "document"),
            unit(3, "后", 0.6, "document"),
            unit(1, "前", 0.5, "document"),
        ];
        let coords = coords_of(&[(1, 1, 0, 100), (2, 1, 100, 200), (3, 1, 200, 300)]);
        let (out, groups, absorbed) = stitch_adjacent(units, &coords);
        assert_eq!((groups, absorbed), (1, 2));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].unit_id, 2);
        assert_eq!(out[0].score, 0.8);
        assert_eq!(out[0].content, "前中后");
    }
}

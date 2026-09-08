//! S3 fusion + S4 cut — RRF over the two recall paths, then top-k
//! [抄EXT:Qdrant hybrid query（RRF，k=60 默认值同）].

use crate::kb::pipeline::search::RawHits;

/// RRF constant `k`, Qdrant's default [抄EXT:Qdrant hybrid].
const RRF_K: f32 = 60.0;

/// Best achievable RRF score: rank #1 in both paths = 2/(k+1). Fused scores
/// are normalized by it so S11's 0..1 threshold (default 0.3 ≈ "not even a
/// solid single-path hit") has comparable semantics across queries.
fn rrf_ceiling() -> f32 {
    2.0 / (RRF_K + 1.0)
}

/// Score fused candidate (pre-hydration: chunk fetched later in bulk).
#[derive(Debug, Clone)]
pub struct FusedCandidate {
    pub unit_id: i64,
    pub score: f32,
}

/// Fuse both hit lists with RRF and keep the top `top_k` units.
pub fn fuse_and_cut(hits: RawHits, top_k: u32) -> Vec<FusedCandidate> {
    let mut scores: std::collections::HashMap<i64, f32> = std::collections::HashMap::new();
    for (rank, (id, _)) in hits.bm25.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
    }
    for (rank, (id, _)) in hits.dense.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
    }
    let ceiling = rrf_ceiling();
    let mut fused: Vec<FusedCandidate> = scores
        .into_iter()
        .map(|(unit_id, score)| FusedCandidate {
            unit_id,
            score: (score / ceiling).clamp(0.0, 1.0),
        })
        .collect();
    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(top_k as usize);
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_paths_agree_ranks_first_and_normalizes() {
        let hits = RawHits {
            bm25: vec![(1, 9.0), (2, 8.0)],
            dense: vec![(1, 0.9), (3, 0.8)],
        };
        let fused = fuse_and_cut(hits, 10);
        assert_eq!(fused[0].unit_id, 1, "dual-path unit must fuse to the top");
        assert!(
            (fused[0].score - 1.0).abs() < 1e-6,
            "rank-1 in both paths normalizes to 1.0, got {}",
            fused[0].score
        );
        assert!(fused.iter().any(|c| c.unit_id == 2));
        assert!(fused.iter().any(|c| c.unit_id == 3));
    }

    #[test]
    fn top_k_truncates() {
        let hits = RawHits {
            bm25: (1..=20).map(|i| (i, i as f32)).collect(),
            dense: Vec::new(),
        };
        let fused = fuse_and_cut(hits, 5);
        assert_eq!(fused.len(), 5);
    }
}

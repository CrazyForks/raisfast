//! In-memory brute-force cosine index — zero-dependency backend.
//!
//! Official-library scale (≤100k vectors) scans in <10ms (technical design
//! §4.2). State lives per process; SQL remains the source of truth and the
//! M2 ingest path re-populates via `rebuild`/`upsert` on boot and re-parse.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{VectorHit, VectorIndex, VectorItem};
use crate::errors::app_error::{AppError, AppResult};

struct StoredVector {
    unit_id: i64,
    kind: String,
    embedding: Vec<f32>,
}

impl StoredVector {
    fn cosine(&self, query: &[f32]) -> f32 {
        if self.embedding.len() != query.len() {
            return f32::NEG_INFINITY;
        }
        let mut dot = 0.0_f32;
        let mut na = 0.0_f32;
        let mut nb = 0.0_f32;
        for (a, b) in self.embedding.iter().zip(query.iter()) {
            dot += a * b;
            na += a * a;
            nb += b * b;
        }
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Brute-force backend: `RwLock<HashMap<kb_id, Vec<StoredVector>>>`.
pub struct BruteForceIndex {
    by_kb: RwLock<HashMap<i64, Vec<Arc<StoredVector>>>>,
}

impl BruteForceIndex {
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_kb: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for BruteForceIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn check_dim(items: &[VectorItem], dim: u32) -> AppResult<()> {
    if let Some(bad) = items.iter().find(|i| i.embedding.len() as u32 != dim) {
        return Err(AppError::BadRequest(format!(
            "vector dim mismatch for unit {}: expected {dim}, got {}",
            bad.unit_id,
            bad.embedding.len()
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
impl VectorIndex for BruteForceIndex {
    async fn upsert(&self, kb_id: i64, dim: u32, items: &[VectorItem]) -> AppResult<()> {
        check_dim(items, dim)?;
        let mut map = self.by_kb.write().await;
        let bucket = map.entry(kb_id).or_default();
        for item in items {
            match bucket.iter_mut().find(|v| v.unit_id == item.unit_id) {
                Some(existing) => {
                    *existing = Arc::new(StoredVector {
                        unit_id: item.unit_id,
                        kind: item.kind.clone(),
                        embedding: item.embedding.clone(),
                    });
                }
                None => bucket.push(Arc::new(StoredVector {
                    unit_id: item.unit_id,
                    kind: item.kind.clone(),
                    embedding: item.embedding.clone(),
                })),
            }
        }
        Ok(())
    }

    async fn delete(&self, kb_id: i64, unit_ids: &[i64]) -> AppResult<()> {
        let mut map = self.by_kb.write().await;
        if let Some(bucket) = map.get_mut(&kb_id) {
            bucket.retain(|v| !unit_ids.contains(&v.unit_id));
        }
        Ok(())
    }

    async fn delete_all(&self, kb_id: i64) -> AppResult<()> {
        self.by_kb.write().await.remove(&kb_id);
        Ok(())
    }

    async fn search(
        &self,
        kb_id: i64,
        embedding: &[f32],
        top_k: usize,
        kind: Option<&str>,
    ) -> AppResult<Vec<VectorHit>> {
        let map = self.by_kb.read().await;
        let Some(bucket) = map.get(&kb_id) else {
            return Ok(Vec::new());
        };
        let mut hits: Vec<VectorHit> = bucket
            .iter()
            .filter(|v| kind.is_none_or(|k| v.kind == k))
            .map(|v| VectorHit {
                unit_id: v.unit_id,
                score: v.cosine(embedding),
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(top_k);
        Ok(hits)
    }

    async fn rebuild(&self, kb_id: i64, dim: u32, items: &[VectorItem]) -> AppResult<()> {
        check_dim(items, dim)?;
        let stored = items
            .iter()
            .map(|i| {
                Arc::new(StoredVector {
                    unit_id: i.unit_id,
                    kind: i.kind.clone(),
                    embedding: i.embedding.clone(),
                })
            })
            .collect();
        self.by_kb.write().await.insert(kb_id, stored);
        Ok(())
    }

    fn backend_name(&self) -> &str {
        "bruteforce"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(unit_id: i64, kb_id: i64, kind: &str, embedding: Vec<f32>) -> VectorItem {
        VectorItem {
            unit_id,
            kb_id,
            kind: kind.to_string(),
            embedding,
        }
    }

    #[tokio::test]
    async fn upsert_search_delete_roundtrip() {
        let idx = BruteForceIndex::new();
        idx.upsert(
            1,
            2,
            &[
                item(10, 1, "document", vec![1.0, 0.0]),
                item(11, 1, "wiki_page", vec![0.0, 1.0]),
                item(12, 1, "document", vec![0.9, 0.1]),
            ],
        )
        .await
        .unwrap();

        let hits = idx.search(1, &[1.0, 0.0], 3, None).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].unit_id, 10, "exact direction must rank first");

        // kind filter
        let wiki = idx
            .search(1, &[1.0, 0.0], 3, Some("wiki_page"))
            .await
            .unwrap();
        assert_eq!(wiki.len(), 1);
        assert_eq!(wiki[0].unit_id, 11);

        // delete removes only the given units
        idx.delete(1, &[10]).await.unwrap();
        let hits = idx.search(1, &[1.0, 0.0], 3, None).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].unit_id, 12);
    }

    #[tokio::test]
    async fn kb_isolation_and_rebuild() {
        let idx = BruteForceIndex::new();
        idx.upsert(1, 2, &[item(10, 1, "document", vec![1.0, 0.0])])
            .await
            .unwrap();
        idx.upsert(2, 2, &[item(20, 2, "document", vec![1.0, 0.0])])
            .await
            .unwrap();

        assert_eq!(idx.search(1, &[1.0, 0.0], 10, None).await.unwrap().len(), 1);
        assert_eq!(idx.search(2, &[1.0, 0.0], 10, None).await.unwrap().len(), 1);

        // rebuild replaces the whole KB bucket
        idx.rebuild(1, 2, &[item(99, 1, "faq", vec![0.0, 1.0])])
            .await
            .unwrap();
        let hits = idx.search(1, &[0.0, 1.0], 10, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].unit_id, 99);

        // delete_all empties it
        idx.delete_all(1).await.unwrap();
        assert!(
            idx.search(1, &[0.0, 1.0], 10, None)
                .await
                .unwrap()
                .is_empty()
        );
        // other KB untouched
        assert_eq!(idx.search(2, &[1.0, 0.0], 10, None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn dim_mismatch_rejected() {
        let idx = BruteForceIndex::new();
        let err = idx
            .upsert(1, 2, &[item(1, 1, "document", vec![1.0, 0.0, 0.0])])
            .await;
        assert!(err.is_err());
    }
}

//! Qdrant backend — default vector index (technical design §4.2).
//!
//! One collection per KB named `{prefix}_{kb_id}`, matching the trait's
//! kb_id scoping (`delete_all`/`rebuild` map to collection drop+recreate)
//! and letting each KB pin its own embedding dimension. Point id = unit
//! snowflake; payload carries `kb_id` and `kind` for filtered searches
//! [抄EXT:qdrant rust-client builders].

use qdrant_client::qdrant::{
    CollectionOperationResponse, Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance,
    Filter, PointStruct, QueryPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::{Payload, Qdrant, QdrantError};

use super::{VectorHit, VectorIndex, VectorItem};
use crate::errors::app_error::{AppError, AppResult};

pub struct QdrantIndex {
    client: Qdrant,
    prefix: String,
}

fn qerr(context: &str, e: QdrantError) -> AppError {
    AppError::ServiceUnavailable(format!("qdrant {context}: {e}"))
}

/// Whether a Qdrant error means "collection/point not found" — used to make
/// reads and deletes idempotent against lazily-created collections.
fn is_not_found(e: &QdrantError) -> bool {
    let msg = e.to_string();
    msg.contains("doesn't exist") || msg.contains("Not found") || msg.contains("not found")
}

impl QdrantIndex {
    /// `url` is the gRPC endpoint (default port 6334), e.g.
    /// `http://localhost:6334`. `api_key` is optional (self-hosted no-auth).
    pub fn new(url: &str, api_key: Option<String>, prefix: &str) -> Result<Self, QdrantError> {
        let mut builder = Qdrant::from_url(url);
        if let Some(key) = api_key {
            builder = builder.api_key(key);
        }
        Ok(Self {
            client: builder.build()?,
            prefix: prefix.to_string(),
        })
    }

    fn collection(&self, kb_id: i64) -> String {
        format!("{}_{}", self.prefix, kb_id)
    }

    async fn ensure_collection(&self, kb_id: i64, dim: u32) -> AppResult<()> {
        let name = self.collection(kb_id);
        if self
            .client
            .collection_exists(&name)
            .await
            .map_err(|e| qerr("exists", e))?
        {
            return Ok(());
        }
        let res: CollectionOperationResponse = self
            .client
            .create_collection(
                CreateCollectionBuilder::new(&name)
                    .vectors_config(VectorParamsBuilder::new(u64::from(dim), Distance::Cosine)),
            )
            .await
            .map_err(|e| qerr("create_collection", e))?;
        if !res.result {
            return Err(AppError::ServiceUnavailable(format!(
                "qdrant create_collection {name} returned result=false"
            )));
        }
        Ok(())
    }

    fn point(item: &VectorItem) -> Result<PointStruct, AppError> {
        let payload: Payload = serde_json::json!({
            "kb_id": item.kb_id,
            "kind": item.kind,
        })
        .try_into()
        .map_err(|e| AppError::BadRequest(format!("payload convert: {e}")))?;
        Ok(PointStruct::new(
            u64::try_from(item.unit_id).unwrap_or_default(),
            item.embedding.clone(),
            payload,
        ))
    }
}

#[async_trait::async_trait]
impl VectorIndex for QdrantIndex {
    async fn upsert(&self, kb_id: i64, dim: u32, items: &[VectorItem]) -> AppResult<()> {
        if items.is_empty() {
            return Ok(());
        }
        self.ensure_collection(kb_id, dim).await?;
        let points = items
            .iter()
            .map(Self::point)
            .collect::<Result<Vec<_>, _>>()?;
        let name = self.collection(kb_id);
        self.client
            .upsert_points(qdrant_client::qdrant::UpsertPointsBuilder::new(
                &name, points,
            ))
            .await
            .map_err(|e| qerr("upsert", e))?;
        Ok(())
    }

    async fn delete(&self, kb_id: i64, unit_ids: &[i64]) -> AppResult<()> {
        if unit_ids.is_empty() {
            return Ok(());
        }
        let name = self.collection(kb_id);
        let ids = unit_ids
            .iter()
            .map(|id| u64::try_from(*id).unwrap_or_default())
            .collect::<Vec<_>>();
        match self
            .client
            .delete_points(DeletePointsBuilder::new(&name).points(ids))
            .await
        {
            Ok(_) => Ok(()),
            // Deleting from a lazily-not-yet-created collection is a no-op.
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(qerr("delete_points", e)),
        }
    }

    async fn delete_all(&self, kb_id: i64) -> AppResult<()> {
        let name = self.collection(kb_id);
        match self.client.delete_collection(&name).await {
            Ok(_) => Ok(()),
            // Dropping a non-existent collection is a successful wipe.
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(qerr("delete_collection", e)),
        }
    }

    async fn search(
        &self,
        kb_id: i64,
        embedding: &[f32],
        top_k: usize,
        kind: Option<&str>,
    ) -> AppResult<Vec<VectorHit>> {
        let name = self.collection(kb_id);
        let mut builder = QueryPointsBuilder::new(&name)
            .query(embedding.to_vec())
            .limit(top_k as u64)
            .with_payload(false);
        if let Some(kind) = kind {
            builder = builder.filter(Filter::all([Condition::matches("kind", kind.to_string())]));
        }
        let result = match self.client.query(builder).await {
            Ok(result) => result,
            // A KB whose collection was never created (or wiped by
            // `delete_all`) simply has nothing to find.
            Err(e) if is_not_found(&e) => return Ok(Vec::new()),
            Err(e) => return Err(qerr("query", e)),
        };
        Ok(result
            .result
            .into_iter()
            .map(|p| VectorHit {
                unit_id: p.id.map_or(i64::MAX, |id| match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(n)) => n as i64,
                    _ => i64::MAX,
                }),
                score: p.score,
            })
            .collect())
    }

    async fn rebuild(&self, kb_id: i64, dim: u32, items: &[VectorItem]) -> AppResult<()> {
        self.delete_all(kb_id).await?;
        self.upsert(kb_id, dim, items).await
    }

    fn backend_name(&self) -> &str {
        "qdrant"
    }

    /// DR9: exact point count for the KB's collection. A missing
    /// collection counts as 0 (lazily created on first upsert).
    async fn count(&self, kb_id: i64) -> AppResult<u64> {
        let name = self.collection(kb_id);
        match self
            .client
            .count(qdrant_client::qdrant::CountPointsBuilder::new(&name).exact(true))
            .await
        {
            Ok(res) => Ok(res.result.map_or(0, |r| r.count)),
            Err(e) if is_not_found(&e) => Ok(0),
            Err(e) => Err(qerr("count", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_if_no_qdrant() -> Option<QdrantIndex> {
        let url = std::env::var("RAISFAST_KB_QDRANT_URL").ok()?;
        QdrantIndex::new(
            &url,
            std::env::var("RAISFAST_KB_QDRANT_API_KEY").ok(),
            "kbtest",
        )
        .ok()
    }

    fn item(unit_id: i64, kb_id: i64, kind: &str, embedding: Vec<f32>) -> VectorItem {
        VectorItem {
            unit_id,
            kb_id,
            kind: kind.to_string(),
            embedding,
        }
    }

    #[tokio::test]
    async fn qdrant_roundtrip() {
        let Some(idx) = skip_if_no_qdrant() else {
            eprintln!("skipping: RAISFAST_KB_QDRANT_URL not set");
            return;
        };
        // Unique KB id per run to avoid collisions on shared instances.
        let kb_id = chrono::Utc::now().timestamp_subsec_nanos() as i64;

        idx.rebuild(
            kb_id,
            2,
            &[
                item(10, kb_id, "document", vec![1.0, 0.0]),
                item(11, kb_id, "wiki_page", vec![0.0, 1.0]),
            ],
        )
        .await
        .unwrap();

        let hits = idx.search(kb_id, &[1.0, 0.0], 2, None).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].unit_id, 10);

        let wiki = idx
            .search(kb_id, &[1.0, 0.0], 2, Some("wiki_page"))
            .await
            .unwrap();
        assert_eq!(wiki.len(), 1);
        assert_eq!(wiki[0].unit_id, 11);

        idx.delete(kb_id, &[10]).await.unwrap();
        let hits = idx.search(kb_id, &[1.0, 0.0], 2, None).await.unwrap();
        assert_eq!(hits.len(), 1);

        idx.delete_all(kb_id).await.unwrap();
        assert!(
            idx.search(kb_id, &[1.0, 0.0], 2, None)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

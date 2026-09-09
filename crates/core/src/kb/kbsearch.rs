//! KB full-text index — a tantivy index separate from the posts index
//! (kb-technical-design §4.3).
//!
//! Implementation mirrors `search/tantivy.rs` (writer mutex +
//! spawn_blocking + delete_term/add/commit) [抄RF:同文件模式]. The schema is
//! KB-specific: unit_id is stored and returned to callers; kb_id, doc_id
//! and kind are raw-tokenized facets for filtering and deletion; text uses
//! the Ngram(2,5)+LowerCaser CJK tokenizer shared with the posts index.
//! SQL stays the content source of truth — this index only maps queries
//! to unit ids plus scores.

use std::path::Path;
use std::sync::Arc;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::document::Value as _;
use tantivy::schema::{Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions};
use tantivy::{Index, IndexReader, IndexWriter, TantivyDocument, Term};
use tokio::sync::Mutex;

use crate::errors::app_error::{AppError, AppResult};

/// A unit to index (id + facet fields + searchable text).
#[derive(Debug, Clone)]
pub struct KbIndexUnit {
    pub unit_id: i64,
    pub kb_id: i64,
    pub doc_id: i64,
    pub kind: String,
    pub text: String,
}

/// One search hit: unit id plus BM25 score.
#[derive(Debug, Clone)]
pub struct KbSearchHit {
    pub unit_id: i64,
    pub score: f32,
}

#[derive(Clone)]
struct FieldSet {
    unit_id: Field,
    kb_id: Field,
    doc_id: Field,
    kind: Field,
    text: Field,
}

fn build_schema() -> (Schema, FieldSet) {
    let mut builder = Schema::builder();
    let raw = TextFieldIndexing::default()
        .set_tokenizer("raw")
        .set_index_option(IndexRecordOption::Basic);
    let facet = |b: &mut tantivy::schema::SchemaBuilder, name: &str| {
        b.add_text_field(
            name,
            TextOptions::default().set_indexing_options(raw.clone()),
        )
    };
    let kb_id = facet(&mut builder, "kb_id");
    let doc_id = facet(&mut builder, "doc_id");
    let kind = facet(&mut builder, "kind");
    let ngram = TextFieldIndexing::default()
        .set_tokenizer("ngram_tokenizer")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text = builder.add_text_field("text", TextOptions::default().set_indexing_options(ngram));
    let unit_stored = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("raw")
                .set_index_option(IndexRecordOption::Basic),
        )
        .set_stored();
    let unit_id = builder.add_text_field("unit_id", unit_stored);
    (
        builder.build(),
        FieldSet {
            unit_id,
            kb_id,
            doc_id,
            kind,
            text,
        },
    )
}

/// KB tantivy engine over a directory (or in memory for tests).
pub struct KbSearchEngine {
    index: Index,
    reader: IndexReader,
    writer: Arc<Mutex<IndexWriter>>,
    fields: FieldSet,
}

impl KbSearchEngine {
    /// Open (or create) the on-disk index at `dir` [抄RF:search/tantivy.rs open].
    pub fn open(dir: impl AsRef<Path>) -> AppResult<Self> {
        let (schema, fields) = build_schema();
        let path = dir.as_ref();
        let index = if path.exists() && path.read_dir().is_ok_and(|mut d| d.next().is_some()) {
            Index::open_in_dir(path).map_err(map_tantivy)?
        } else {
            std::fs::create_dir_all(path)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("kb index mkdir: {e}")))?;
            Index::create_in_dir(path, schema).map_err(map_tantivy)?
        };
        Self::from_index(index, fields)
    }

    /// In-memory index (tests / no persistence needed).
    pub fn open_in_memory() -> AppResult<Self> {
        let (schema, fields) = build_schema();
        Self::from_index(Index::create_in_ram(schema), fields)
    }

    fn from_index(index: Index, fields: FieldSet) -> AppResult<Self> {
        // Ngram 2-5 + LowerCaser, same CJK approach as the posts index
        // [抄RF:search/tantivy.rs].
        // Same CJK tokenizer registration as the posts index
        // [copy RF:search/tantivy.rs register_tokenizers].
        let ngram = tantivy::tokenizer::TextAnalyzer::builder(
            tantivy::tokenizer::NgramTokenizer::all_ngrams(2, 5)
                .unwrap_or_else(|e| panic!("invalid ngram params: {e}")),
        )
        .filter(tantivy::tokenizer::LowerCaser)
        .build();
        index.tokenizers().register("ngram_tokenizer", ngram);
        let writer = index
            .writer::<TantivyDocument>(15_000_000)
            .map_err(map_tantivy)?;
        let reader = index.reader().map_err(map_tantivy)?;
        Ok(Self {
            index,
            reader,
            writer: Arc::new(Mutex::new(writer)),
            fields,
        })
    }

    fn doc(&self, u: &KbIndexUnit) -> TantivyDocument {
        let mut d = TantivyDocument::new();
        d.add_text(self.fields.unit_id, u.unit_id.to_string());
        d.add_text(self.fields.kb_id, u.kb_id.to_string());
        d.add_text(self.fields.doc_id, u.doc_id.to_string());
        d.add_text(self.fields.kind, &u.kind);
        d.add_text(self.fields.text, &u.text);
        d
    }

    /// Replace the indexed units of one document (idempotency: delete by
    /// doc facet, then add) [抄RF:search_index.rs delete_term+add+commit].
    pub async fn reindex_document(&self, units: &[KbIndexUnit]) -> AppResult<()> {
        let Some(first) = units.first() else {
            return Ok(());
        };
        let doc_term = Term::from_field_text(self.fields.doc_id, &first.doc_id.to_string());
        let docs: Vec<TantivyDocument> = units.iter().map(|u| self.doc(u)).collect();
        let writer = self.writer.clone();
        tokio::task::spawn_blocking(move || {
            let mut w = writer.blocking_lock();
            w.delete_term(doc_term);
            for d in docs {
                w.add_document(d).map_err(map_tantivy)?;
            }
            w.commit().map_err(map_tantivy)?;
            Ok::<(), AppError>(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("join: {e}")))??;
        self.reader.reload().map_err(map_tantivy)?;
        Ok(())
    }

    /// Remove all units of one document from the index.
    pub async fn delete_document(&self, doc_id: i64) -> AppResult<()> {
        let term = Term::from_field_text(self.fields.doc_id, &doc_id.to_string());
        let writer = self.writer.clone();
        tokio::task::spawn_blocking(move || {
            let mut w = writer.blocking_lock();
            w.delete_term(term);
            w.commit().map_err(map_tantivy)?;
            Ok::<(), AppError>(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("join: {e}")))??;
        self.reader.reload().map_err(map_tantivy)?;
        Ok(())
    }

    /// Remove every unit of one knowledge base from the index (KB delete).
    pub async fn delete_kb(&self, kb_id: i64) -> AppResult<()> {
        let term = Term::from_field_text(self.fields.kb_id, &kb_id.to_string());
        let writer = self.writer.clone();
        tokio::task::spawn_blocking(move || {
            let mut w = writer.blocking_lock();
            w.delete_term(term);
            w.commit().map_err(map_tantivy)?;
            Ok::<(), AppError>(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("join: {e}")))??;
        self.reader.reload().map_err(map_tantivy)?;
        Ok(())
    }

    /// BM25 search within one KB. Chinese-friendly via the ngram tokenizer.
    pub async fn search(
        &self,
        kb_id: i64,
        query: &str,
        top_n: usize,
    ) -> AppResult<Vec<KbSearchHit>> {
        let index = self.index.clone();
        let kb_field = self.fields.kb_id;
        let unit_field = self.fields.unit_id;
        let text_field = self.fields.text;
        let reader = self.reader.clone();
        let query = query.to_string();

        tokio::task::spawn_blocking(move || -> AppResult<Vec<KbSearchHit>> {
            let searcher = reader.searcher();
            let kb_term = tantivy::query::TermQuery::new(
                Term::from_field_text(kb_field, &kb_id.to_string()),
                // facet fields are Basic-indexed
                tantivy::schema::IndexRecordOption::Basic,
            );
            let text_qp = QueryParser::for_index(&index, vec![text_field]);
            let parsed = text_qp
                .parse_query(&query)
                .map_err(|e| AppError::BadRequest(format!("invalid kb query: {e}")))?;
            let filtered = tantivy::query::BooleanQuery::new(vec![
                (tantivy::query::Occur::Must, Box::new(kb_term)),
                (tantivy::query::Occur::Must, Box::new(parsed)),
            ]);
            let top: Vec<(tantivy::Score, tantivy::DocAddress)> = searcher
                .search(&filtered, &TopDocs::with_limit(top_n).order_by_score())
                .map_err(map_tantivy)?;
            let mut hits = Vec::with_capacity(top.len());
            for (score, addr) in top {
                let doc: TantivyDocument = searcher.doc(addr).map_err(map_tantivy)?;
                let unit = doc
                    .get_first(unit_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .parse::<i64>()
                    .unwrap_or_default();
                hits.push(KbSearchHit {
                    unit_id: unit,
                    score,
                });
            }
            Ok(hits)
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("join: {e}")))?
    }
}

fn map_tantivy(e: tantivy::TantivyError) -> AppError {
    AppError::Internal(anyhow::anyhow!("tantivy: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reindex_search_delete_roundtrip() {
        let e = KbSearchEngine::open_in_memory().unwrap();
        e.reindex_document(&[
            unit(1, 1, 100, "document", "Rust 所有权系统入门"),
            unit(2, 1, 100, "document", "Go 协程调度"),
        ])
        .await
        .unwrap();
        e.reindex_document(&[unit(3, 1, 101, "wiki_page", "Rust 借用检查")])
            .await
            .unwrap();

        let hits = e.search(1, "所有权", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].unit_id, 1);

        let hits = e.search(1, "Rust", 10).await.unwrap();
        assert_eq!(hits.len(), 2, "both rust units must match");

        // kb isolation
        e.reindex_document(&[unit(9, 2, 200, "document", "Rust 别的库")])
            .await
            .unwrap();
        let hits = e.search(2, "Rust", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].unit_id, 9);

        // doc-level delete removes only that doc
        e.delete_document(100).await.unwrap();
        let hits = e.search(1, "Rust", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].unit_id, 3);
    }

    fn unit(unit_id: i64, kb_id: i64, doc_id: i64, kind: &str, text: &str) -> KbIndexUnit {
        KbIndexUnit {
            unit_id,
            kb_id,
            doc_id,
            kind: kind.to_string(),
            text: text.to_string(),
        }
    }
}

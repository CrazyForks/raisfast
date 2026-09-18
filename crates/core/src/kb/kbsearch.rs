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
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
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

    /// Append units WITHOUT touching the rest of their document. Unlike
    /// `reindex_document` (doc-scoped replace), this only replaces each
    /// passed unit by its own `unit_id` — callers adding a subset of a
    /// document's units (e.g. image captions from the recognize job,
    /// 2026-09-18: it erased the doc's 179 text chunks from the index)
    /// must use this, or bm25 recall for the doc degrades to the subset.
    pub async fn append_units(&self, units: &[KbIndexUnit]) -> AppResult<()> {
        if units.is_empty() {
            return Ok(());
        }
        let docs: Vec<(Term, TantivyDocument)> = units
            .iter()
            .map(|u| {
                (
                    Term::from_field_text(self.fields.unit_id, &u.unit_id.to_string()),
                    self.doc(u),
                )
            })
            .collect();
        let writer = self.writer.clone();
        tokio::task::spawn_blocking(move || {
            let mut w = writer.blocking_lock();
            for (unit_term, d) in &docs {
                w.delete_term(unit_term.clone());
                w.add_document(d.clone()).map_err(map_tantivy)?;
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

    /// Count indexed units per document within one KB (index viewer).
    /// Committed-state exact counts via the Count collector, one term pair
    /// per doc — doc counts are small, and facets are not stored so a
    /// group-by is not available.
    pub async fn doc_counts(
        &self,
        kb_id: i64,
        doc_ids: &[i64],
    ) -> AppResult<std::collections::HashMap<i64, u64>> {
        let kb_field = self.fields.kb_id;
        let doc_field = self.fields.doc_id;
        let reader = self.reader.clone();
        let doc_ids = doc_ids.to_vec();
        tokio::task::spawn_blocking(move || -> AppResult<std::collections::HashMap<i64, u64>> {
            let searcher = reader.searcher();
            let mut out = std::collections::HashMap::new();
            for doc_id in doc_ids {
                let q = BooleanQuery::new(vec![
                    (
                        Occur::Must,
                        Box::new(TermQuery::new(
                            Term::from_field_text(kb_field, &kb_id.to_string()),
                            IndexRecordOption::Basic,
                        )) as Box<dyn Query>,
                    ),
                    (
                        Occur::Must,
                        Box::new(TermQuery::new(
                            Term::from_field_text(doc_field, &doc_id.to_string()),
                            IndexRecordOption::Basic,
                        )) as Box<dyn Query>,
                    ),
                ]);
                let n = searcher
                    .search(&q, &tantivy::collector::Count)
                    .map_err(map_tantivy)?;
                out.insert(doc_id, u64::try_from(n).unwrap_or(0));
            }
            Ok(out)
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("join: {e}")))?
    }

    /// Total indexed unit count for one KB (index viewer drift totals).
    pub async fn count_kb(&self, kb_id: i64) -> AppResult<u64> {
        let kb_field = self.fields.kb_id;
        let reader = self.reader.clone();
        tokio::task::spawn_blocking(move || -> AppResult<u64> {
            let searcher = reader.searcher();
            let q = TermQuery::new(
                Term::from_field_text(kb_field, &kb_id.to_string()),
                IndexRecordOption::Basic,
            );
            let n = searcher
                .search(&q, &tantivy::collector::Count)
                .map_err(map_tantivy)?;
            Ok(u64::try_from(n).unwrap_or(0))
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("join: {e}")))?
    }

    /// Analyze text with the text field's tokenizer — the indexed keyword
    /// space of that text (all n-grams, deduped, in token order).
    pub async fn analyze(&self, text: &str) -> AppResult<Vec<String>> {
        let index = self.index.clone();
        let text_field = self.fields.text;
        let text = text.to_string();
        tokio::task::spawn_blocking(move || -> AppResult<Vec<String>> {
            let mut tokenizer = index.tokenizer_for_field(text_field).map_err(map_tantivy)?;
            let mut out: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut stream = tokenizer.token_stream(&text);
            while let Some(token) = stream.next() {
                let gram = token.text.to_string();
                if seen.insert(gram.clone()) {
                    out.push(gram);
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("join: {e}")))?
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
    ///
    /// The query is analyzed with the field's own tokenizer and the produced
    /// grams are OR-ed (minimum one). `QueryParser` is deliberately not used:
    /// it conjoins the n-grams of a single space-free term, so a mixed
    /// latin+CJK question like "solana是什么" required every gram to hit —
    /// no document can — and BM25 recall silently returned nothing for such
    /// questions (2026-09-18). An OR of grams keeps recall; BM25 idf ranks
    /// longer (rarer) grams higher.
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
            let kb_term = TermQuery::new(
                Term::from_field_text(kb_field, &kb_id.to_string()),
                IndexRecordOption::Basic,
            );
            let mut tokenizer = index.tokenizer_for_field(text_field).map_err(map_tantivy)?;
            let mut grams: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut stream = tokenizer.token_stream(&query);
            while let Some(token) = stream.next() {
                let gram = token.text.to_string();
                if seen.insert(gram.clone()) {
                    grams.push(gram);
                }
            }
            if grams.is_empty() {
                return Ok(Vec::new());
            }
            let mut subqueries: Vec<(Occur, Box<dyn Query>)> =
                vec![(Occur::Must, Box::new(kb_term))];
            // Inner OR (pure-Should boolean requires ≥1 match) over all grams.
            let gram_shoulds: Vec<(Occur, Box<dyn Query>)> = grams
                .into_iter()
                .map(|gram| {
                    let term_query = TermQuery::new(
                        Term::from_field_text(text_field, &gram),
                        IndexRecordOption::WithFreqsAndPositions,
                    );
                    (Occur::Should, Box::new(term_query) as Box<dyn Query>)
                })
                .collect();
            subqueries.push((Occur::Must, Box::new(BooleanQuery::new(gram_shoulds))));
            let filtered = BooleanQuery::new(subqueries);
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

    /// Append-only indexing (image-caption path): adding a doc's image units
    /// must NOT erase the doc's text units — `reindex_document` did exactly
    /// that (2026-09-18: 179 text chunks wiped, bm25 degraded to captions).
    #[tokio::test]
    async fn append_units_keeps_doc_text_units() {
        let e = KbSearchEngine::open_in_memory().unwrap();
        e.reindex_document(&[
            unit(
                1,
                1,
                100,
                "document",
                "Solana is a high performance blockchain",
            ),
            unit(
                2,
                1,
                100,
                "document",
                "second text chunk about proof of history",
            ),
        ])
        .await
        .unwrap();

        // The image-recognition job appends caption units of the SAME doc.
        e.append_units(&[
            unit(3, 1, 100, "image", "该图展示了事务处理流程"),
            unit(4, 1, 100, "image", "该图展示了验证器网络"),
        ])
        .await
        .unwrap();

        // Text units survived the append…
        let hits = e.search(1, "blockchain", 10).await.unwrap();
        assert!(
            hits.iter().any(|h| h.unit_id == 1),
            "text unit must survive"
        );
        // …and the caption units are searchable too.
        let hits = e.search(1, "该图展示了", 10).await.unwrap();
        assert_eq!(hits.len(), 2, "both caption units must be indexed");
        // Re-appending the same units stays idempotent (no duplicates).
        e.append_units(&[unit(3, 1, 100, "image", "该图展示了事务处理流程")])
            .await
            .unwrap();
        let hits = e.search(1, "该图展示了", 10).await.unwrap();
        assert_eq!(hits.len(), 2, "append must stay idempotent per unit_id");
    }

    /// Mixed latin+CJK query without spaces must still recall (ngram grams
    /// are OR-ed, not conjoined) — the 2026-09-18 bm25-empty regression.
    #[tokio::test]
    async fn mixed_script_query_recalls() {
        let e = KbSearchEngine::open_in_memory().unwrap();
        e.reindex_document(&[
            unit(
                1,
                1,
                100,
                "document",
                "Solana is a high performance blockchain platform",
            ),
            unit(2, 1, 100, "document", "今天天气很好适合散步"),
        ])
        .await
        .unwrap();

        let hits = e.search(1, "solana是什么", 10).await.unwrap();
        assert_eq!(
            hits[0].unit_id, 1,
            "latin grams must recall the english doc"
        );

        let hits = e.search(1, "solana", 10).await.unwrap();
        assert_eq!(hits[0].unit_id, 1);

        // Pure CJK keeps working, and kb isolation is intact.
        let hits = e.search(1, "天气很好", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].unit_id, 2);

        // Grams matching nothing stay empty rather than erroring.
        let hits = e.search(1, "zzzzzz", 10).await.unwrap();
        assert!(hits.is_empty());
    }
}

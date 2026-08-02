//! Full-text search engine implementation based on Tantivy 0.26

use std::sync::{Arc, Mutex};

use tantivy::TantivyDocument;
use tantivy::collector::Count;
use tantivy::doc;
use tantivy::schema::{IndexRecordOption, SchemaBuilder, TextFieldIndexing, TextOptions, Value};

use super::{SearchEngine, SearchResult, SearchablePost};

use crate::errors::app_error::{AppError, AppResult};

struct FieldSet {
    post_id: tantivy::schema::Field,
    title: tantivy::schema::Field,
    content: tantivy::schema::Field,
}

pub struct TantivyEngine {
    index: tantivy::Index,
    fields: FieldSet,
    writer: Arc<Mutex<tantivy::IndexWriter<TantivyDocument>>>,
}

impl TantivyEngine {
    pub fn open(index_dir: &str) -> AppResult<Self> {
        let schema = Self::build_schema();
        let post_id = schema.get_field("post_id").unwrap();
        let title = schema.get_field("title").unwrap();
        let content = schema.get_field("content").unwrap();

        let path = std::path::Path::new(index_dir);
        let index = if path.exists() && path.read_dir().is_ok_and(|mut d| d.next().is_some()) {
            tantivy::Index::open_in_dir(path)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("open index: {e}")))?
        } else {
            std::fs::create_dir_all(path)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir: {e}")))?;
            tantivy::Index::create_in_dir(path, schema)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("create index: {e}")))?
        };

        Self::register_tokenizers(&index);
        let writer = index
            .writer::<TantivyDocument>(15_000_000)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("writer: {e}")))?;
        Ok(Self {
            index,
            fields: FieldSet {
                post_id,
                title,
                content,
            },
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    pub fn open_in_memory() -> AppResult<Self> {
        let schema = Self::build_schema();
        let post_id = schema.get_field("post_id").unwrap();
        let title = schema.get_field("title").unwrap();
        let content = schema.get_field("content").unwrap();

        let index = tantivy::Index::create_in_ram(schema);
        Self::register_tokenizers(&index);
        let writer = index
            .writer::<TantivyDocument>(15_000_000)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("writer: {e}")))?;
        Ok(Self {
            index,
            fields: FieldSet {
                post_id,
                title,
                content,
            },
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    fn build_schema() -> tantivy::schema::Schema {
        let mut builder = SchemaBuilder::new();

        let id_indexing = TextFieldIndexing::default()
            .set_tokenizer("raw")
            .set_index_option(IndexRecordOption::Basic);
        let id_opts = TextOptions::default()
            .set_indexing_options(id_indexing)
            .set_stored();

        let text_indexing = TextFieldIndexing::default()
            .set_tokenizer("ngram")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions);
        let text_opts = TextOptions::default()
            .set_indexing_options(text_indexing)
            .set_stored();

        builder.add_text_field("post_id", id_opts);
        builder.add_text_field("title", text_opts.clone());
        builder.add_text_field("content", text_opts);
        builder.build()
    }

    fn register_tokenizers(index: &tantivy::Index) {
        let ngram = tantivy::tokenizer::TextAnalyzer::builder(
            tantivy::tokenizer::NgramTokenizer::all_ngrams(2, 5)
                .unwrap_or_else(|e| panic!("invalid ngram params: {e}")),
        )
        .filter(tantivy::tokenizer::LowerCaser)
        .build();
        index.tokenizers().register("ngram", ngram);
    }
}

#[async_trait::async_trait]
impl SearchEngine for TantivyEngine {
    fn engine_name(&self) -> &str {
        "tantivy"
    }

    async fn index_post(&self, post: &SearchablePost) -> AppResult<()> {
        let writer = self.writer.clone();
        let f_post_id = self.fields.post_id;
        let f_title = self.fields.title;
        let f_content = self.fields.content;
        let post = post.clone();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut w = writer
                .lock()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("lock: {e}")))?;
            w.delete_term(tantivy::Term::from_field_text(f_post_id, &post.id));
            w.add_document(doc!(
                f_post_id => post.id.as_str(),
                f_title => post.title.as_str(),
                f_content => post.content.as_str(),
            ))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("add: {e}")))?;
            w.commit()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }

    async fn index_posts(&self, posts: &[SearchablePost]) -> AppResult<()> {
        let writer = self.writer.clone();
        let f_post_id = self.fields.post_id;
        let f_title = self.fields.title;
        let f_content = self.fields.content;
        let posts: Vec<SearchablePost> = posts.to_vec();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut w = writer
                .lock()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("lock: {e}")))?;
            for post in &posts {
                w.delete_term(tantivy::Term::from_field_text(f_post_id, post.id.as_str()));
                w.add_document(doc!(
                    f_post_id => post.id.as_str(),
                    f_title => post.title.as_str(),
                    f_content => post.content.as_str(),
                ))
                .map_err(|e| AppError::Internal(anyhow::anyhow!("add: {e}")))?;
            }
            w.commit()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }

    async fn delete_post(&self, post_id: &str) -> AppResult<()> {
        let writer = self.writer.clone();
        let f_post_id = self.fields.post_id;
        let post_id = post_id.to_string();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut w = writer
                .lock()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("lock: {e}")))?;
            w.delete_term(tantivy::Term::from_field_text(f_post_id, &post_id));
            w.commit()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }

    async fn rebuild_all(&self, posts: &[SearchablePost]) -> AppResult<()> {
        let writer = self.writer.clone();
        let f_post_id = self.fields.post_id;
        let f_title = self.fields.title;
        let f_content = self.fields.content;
        let posts: Vec<SearchablePost> = posts.to_vec();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut w = writer
                .lock()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("lock: {e}")))?;
            w.delete_all_documents()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("delete all: {e}")))?;
            for post in &posts {
                w.add_document(doc!(
                    f_post_id => post.id.as_str(),
                    f_title => post.title.as_str(),
                    f_content => post.content.as_str(),
                ))
                .map_err(|e| AppError::Internal(anyhow::anyhow!("add: {e}")))?;
            }
            w.commit()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }

    async fn search(
        &self,
        query: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<SearchResult>, i64)> {
        let index = self.index.clone();
        let f_post_id = self.fields.post_id;
        let f_title = self.fields.title;
        let f_content = self.fields.content;
        let query_str = query.to_string();

        tokio::task::spawn_blocking(move || -> AppResult<(Vec<SearchResult>, i64)> {
            let reader = index
                .reader()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("reader: {e}")))?;
            let searcher = reader.searcher();

            let qp = tantivy::query::QueryParser::for_index(&index, vec![f_title, f_content]);
            let parsed = qp
                .parse_query(&query_str)
                .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;

            let count = searcher
                .search(&*parsed, &Count)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("count: {e}")))?
                as i64;

            let offset = ((page - 1).max(0) * page_size) as usize;
            let limit = page_size as usize;

            let top_docs: Vec<(tantivy::Score, tantivy::DocAddress)> = searcher
                .search(
                    &parsed,
                    &tantivy::collector::TopDocs::with_limit(limit)
                        .and_offset(offset)
                        .order_by_score(),
                )
                .map_err(|e| AppError::Internal(anyhow::anyhow!("search: {e}")))?;

            let mut results = Vec::with_capacity(top_docs.len());
            for (score, addr) in top_docs {
                let doc: tantivy::TantivyDocument = searcher
                    .doc(addr)
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("doc: {e}")))?;

                let post_id = doc
                    .get_first(f_post_id)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let title_text = doc
                    .get_first(f_title)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let content_text = doc
                    .get_first(f_content)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let title_highlight = Some(super::highlight_text(&query_str, &title_text));
                let excerpt = super::make_excerpt(&content_text, &query_str, 200);
                let excerpt_highlight = excerpt.map(|e| super::highlight_text(&query_str, &e));

                results.push(SearchResult {
                    post_id,
                    score,
                    title_highlight,
                    excerpt_highlight,
                });
            }

            Ok((results, count))
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }
}

impl TantivyEngine {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(id: &str, title: &str, content: &str) -> SearchablePost {
        SearchablePost {
            id: id.into(),
            title: title.into(),
            content: content.into(),
        }
    }

    #[tokio::test]
    async fn index_and_search() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp(
            "p1",
            "Rust Programming Introduction",
            "Rust is a safe systems programming language",
        ))
        .await
        .unwrap();
        e.index_post(&sp(
            "p2",
            "Go Language Tutorial",
            "Go is a programming language developed by Google",
        ))
        .await
        .unwrap();
        let (r, t) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn search_chinese() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp(
            "p1",
            "Understanding Rust in Depth",
            "Rust ownership system is its core feature",
        ))
        .await
        .unwrap();
        e.index_post(&sp(
            "p2",
            "Rust in Practice",
            "Learn Rust through practical projects",
        ))
        .await
        .unwrap();
        let (r, t) = e.search("ownership", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn batch_index() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_posts(&[
            sp("p1", "Title One", "Content One"),
            sp("p2", "Title Two", "Content Two"),
            sp("p3", "Title Three", "Content Three"),
        ])
        .await
        .unwrap();
        let (_r, t) = e.search("Title", 1, 10).await.unwrap();
        assert_eq!(t, 3);
    }

    #[tokio::test]
    async fn delete_post() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Rust Getting Started", "Rust Programming"))
            .await
            .unwrap();
        e.index_post(&sp("p2", "Go Getting Started", "Go Programming"))
            .await
            .unwrap();
        e.delete_post("p1").await.unwrap();
        let (r, t) = e.search("getting started", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p2");
    }

    #[tokio::test]
    async fn rebuild_all() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Old Article", "Old Content"))
            .await
            .unwrap();
        e.rebuild_all(&[
            sp("p2", "New Article A", "New Content A"),
            sp("p3", "New Article B", "New Content B"),
        ])
        .await
        .unwrap();
        assert_eq!(e.search("Old", 1, 10).await.unwrap().1, 0);
        assert_eq!(e.search("New Article", 1, 10).await.unwrap().1, 2);
    }

    #[tokio::test]
    async fn pagination() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..5)
            .map(|i| {
                sp(
                    &format!("p{i}"),
                    &format!("Test Article {i}"),
                    &format!("Content {i}"),
                )
            })
            .collect();
        e.index_posts(&posts).await.unwrap();
        assert_eq!(e.search("Test", 1, 2).await.unwrap().1, 5);
        assert_eq!(e.search("Test", 1, 2).await.unwrap().0.len(), 2);
        assert_eq!(e.search("Test", 3, 2).await.unwrap().0.len(), 1);
    }

    #[tokio::test]
    async fn update_existing() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Old Heading", "Old body"))
            .await
            .unwrap();
        e.index_post(&sp("p1", "Fresh Entry", "Fresh body"))
            .await
            .unwrap();
        assert_eq!(e.search("Old Heading", 1, 10).await.unwrap().1, 0);
        assert_eq!(e.search("Fresh Entry", 1, 10).await.unwrap().1, 1);
    }

    #[tokio::test]
    async fn no_results() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Rust Getting Started", "Rust Programming"))
            .await
            .unwrap();
        let (r, t) = e.search("nonexistent xyz", 1, 10).await.unwrap();
        assert_eq!(t, 0);
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn highlight_in_result() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp(
            "p1",
            "Rust Programming Language",
            "Rust is a modern systems programming language",
        ))
        .await
        .unwrap();
        let (r, _) = e.search("Rust", 1, 10).await.unwrap();
        assert!(
            r[0].title_highlight
                .as_ref()
                .unwrap()
                .contains("<em>Rust</em>")
        );
    }

    #[test]
    fn highlight_unit() {
        assert_eq!(
            super::super::highlight_text("Rust", "Learn Rust Programming"),
            "Learn <em>Rust</em> Programming"
        );
    }

    #[test]
    fn highlight_multi_word() {
        let result = super::super::highlight_text(
            "Rust language",
            "Rust language tutorial and Rust practice",
        );
        assert!(result.contains("<em>Rust</em>"));
        assert!(result.contains("<em>language</em>"));
    }

    #[test]
    fn highlight_case_insensitive() {
        let result = super::super::highlight_text("rust", "Learn RUST Programming");
        assert!(result.contains("<em>"));
    }

    #[test]
    fn highlight_no_match() {
        let text = "This is plain text";
        assert_eq!(super::super::highlight_text("xyz", text), text);
    }

    #[test]
    fn excerpt_unit() {
        let c = "This is a long piece of content that contains the keyword Rust, and the rest should be truncated.";
        let e = super::super::make_excerpt(c, "Rust", 50);
        assert!(e.unwrap().contains("Rust"));
    }

    #[test]
    fn excerpt_short_content() {
        let c = "Short content Rust test";
        let e = super::super::make_excerpt(c, "Rust", 200);
        let result = e.unwrap();
        assert!(result.contains("Rust"));
        assert!(!result.starts_with("..."));
    }

    #[test]
    fn excerpt_keyword_at_start() {
        let c = "Rust at the beginning of test content followed by lots of filler text";
        let e = super::super::make_excerpt(c, "Rust", 20);
        assert!(e.unwrap().contains("Rust"));
    }

    #[test]
    fn excerpt_keyword_at_end() {
        let c = "This is some preceding content and the keyword Rust at the end";
        let e = super::super::make_excerpt(c, "Rust", 20);
        assert!(e.unwrap().contains("Rust"));
    }

    #[test]
    fn excerpt_no_match_returns_none() {
        let c = "Text without any keyword";
        assert!(super::super::make_excerpt(c, "xyz", 50).is_none());
    }

    #[test]
    fn excerpt_multi_word_query_uses_first() {
        let c = "Content contains First keyword and Second keyword";
        let e = super::super::make_excerpt(c, "First Second", 50);
        assert!(e.unwrap().contains("First"));
    }

    #[tokio::test]
    async fn is_noop_returns_false() {
        let e = TantivyEngine::open_in_memory().unwrap();
        assert!(!e.is_noop());
    }

    #[tokio::test]
    async fn index_posts_empty_slice() {
        let e = TantivyEngine::open_in_memory().unwrap();
        assert!(e.index_posts(&[]).await.is_ok());
        let (_, total) = e.search("anything", 1, 10).await.unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn rebuild_all_empty() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Article", "Content")).await.unwrap();
        e.rebuild_all(&[]).await.unwrap();
        assert_eq!(e.search("Article", 1, 10).await.unwrap().1, 0);
    }

    #[tokio::test]
    async fn delete_nonexistent_post_ok() {
        let e = TantivyEngine::open_in_memory().unwrap();
        assert!(e.delete_post("nonexistent").await.is_ok());
    }

    #[tokio::test]
    async fn search_returns_score() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Rust programming", "Rust is safe"))
            .await
            .unwrap();
        let (r, _) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].score > 0.0);
    }

    #[tokio::test]
    async fn search_result_has_highlights() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp(
            "p1",
            "Rust Programming Language",
            "Rust is a modern systems programming language",
        ))
        .await
        .unwrap();
        let (r, _) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].title_highlight.is_some());
        assert!(r[0].excerpt_highlight.is_some());
    }

    #[tokio::test]
    async fn search_result_post_id_matches() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("abc-123", "Title", "Content contains keyword match"))
            .await
            .unwrap();
        let (r, _) = e.search("match", 1, 10).await.unwrap();
        assert_eq!(r[0].post_id, "abc-123");
    }

    #[tokio::test]
    async fn search_page_zero_treated_as_page_one() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..5)
            .map(|i| {
                sp(
                    &format!("p{i}"),
                    &format!("Rust{i}"),
                    &format!("Content {i}"),
                )
            })
            .collect();
        e.index_posts(&posts).await.unwrap();
        let (r, t) = e.search("Rust", 0, 2).await.unwrap();
        assert_eq!(t, 5);
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn search_negative_page_treated_as_page_one() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..5)
            .map(|i| {
                sp(
                    &format!("p{i}"),
                    &format!("Rust{i}"),
                    &format!("Content {i}"),
                )
            })
            .collect();
        e.index_posts(&posts).await.unwrap();
        let (r, t) = e.search("Rust", -1, 2).await.unwrap();
        assert_eq!(t, 5);
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn search_invalid_query_returns_error() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let result = e.search("field:invalid[syntax", 1, 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn search_mixed_chinese_english() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp(
            "p1",
            "Rust Programming Getting Started Guide",
            "Learn Rust programming language from scratch",
        ))
        .await
        .unwrap();
        e.index_post(&sp(
            "p2",
            "Python Data Analysis",
            "Using Python for data analysis",
        ))
        .await
        .unwrap();
        let (r, t) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn search_content_field() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp(
            "p1",
            "Plain Title",
            "This content contains the special keyword Titanic",
        ))
        .await
        .unwrap();
        let (r, t) = e.search("Titanic", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn index_large_batch() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..100)
            .map(|i| {
                sp(
                    &format!("p{i}"),
                    &format!("Batch Article {i}"),
                    &format!("Batch Content {i}"),
                )
            })
            .collect();
        e.index_posts(&posts).await.unwrap();
        let (_, t) = e.search("Batch", 1, 10).await.unwrap();
        assert_eq!(t, 100);
    }
}

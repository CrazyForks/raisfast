//! 基于 Tantivy 0.26 的全文搜索引擎实现

use std::path::Path;

use crate::errors::app_error::{AppError, AppResult};
use tantivy::collector::Count;
use tantivy::doc;
use tantivy::schema::{IndexRecordOption, SchemaBuilder, TextFieldIndexing, TextOptions, Value};

use super::{SearchEngine, SearchablePost, SearchResult};

struct FieldSet {
    post_id: tantivy::schema::Field,
    title: tantivy::schema::Field,
    content: tantivy::schema::Field,
}

pub struct TantivyEngine {
    index: tantivy::Index,
    fields: FieldSet,
}

impl TantivyEngine {
    pub fn open(index_dir: &str) -> AppResult<Self> {
        let schema = Self::build_schema();
        let post_id = schema.get_field("post_id").unwrap();
        let title = schema.get_field("title").unwrap();
        let content = schema.get_field("content").unwrap();

        let path = Path::new(index_dir);
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
        Ok(Self { index, fields: FieldSet { post_id, title, content } })
    }

    pub fn open_in_memory() -> AppResult<Self> {
        let schema = Self::build_schema();
        let post_id = schema.get_field("post_id").unwrap();
        let title = schema.get_field("title").unwrap();
        let content = schema.get_field("content").unwrap();

        let index = tantivy::Index::create_in_ram(schema);
        Self::register_tokenizers(&index);
        Ok(Self { index, fields: FieldSet { post_id, title, content } })
    }

    fn build_schema() -> tantivy::schema::Schema {
        let mut builder = SchemaBuilder::new();

        let id_indexing = TextFieldIndexing::default()
            .set_tokenizer("raw")
            .set_index_option(IndexRecordOption::Basic);
        let id_opts = TextOptions::default().set_indexing_options(id_indexing).set_stored();

        let text_indexing = TextFieldIndexing::default()
            .set_tokenizer("ngram")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions);
        let text_opts = TextOptions::default().set_indexing_options(text_indexing).set_stored();

        builder.add_text_field("post_id", id_opts);
        builder.add_text_field("title", text_opts.clone());
        builder.add_text_field("content", text_opts);
        builder.build()
    }

    fn register_tokenizers(index: &tantivy::Index) {
        let ngram = tantivy::tokenizer::TextAnalyzer::builder(
            tantivy::tokenizer::NgramTokenizer::all_ngrams(2, 5)
                .expect("ngram params valid"),
        )
        .filter(tantivy::tokenizer::LowerCaser)
        .build();
        index.tokenizers().register("ngram", ngram);
    }
}

#[async_trait::async_trait]
impl SearchEngine for TantivyEngine {
    async fn index_post(&self, post: &SearchablePost) -> AppResult<()> {
        let index = self.index.clone();
        let f_post_id = self.fields.post_id;
        let f_title = self.fields.title;
        let f_content = self.fields.content;
        let post = post.clone();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut writer = index.writer::<tantivy::TantivyDocument>(15_000_000)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("writer: {e}")))?;
            writer.delete_term(tantivy::Term::from_field_text(f_post_id, &post.id));
            writer.add_document(doc!(
                f_post_id => post.id.as_str(),
                f_title => post.title.as_str(),
                f_content => post.content.as_str(),
            )).map_err(|e| AppError::Internal(anyhow::anyhow!("add: {e}")))?;
            writer.commit().map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        }).await.map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }

    async fn index_posts(&self, posts: &[SearchablePost]) -> AppResult<()> {
        let index = self.index.clone();
        let f_post_id = self.fields.post_id;
        let f_title = self.fields.title;
        let f_content = self.fields.content;
        let posts: Vec<SearchablePost> = posts.to_vec();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut writer = index.writer::<tantivy::TantivyDocument>(15_000_000)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("writer: {e}")))?;
            for post in &posts {
                writer.delete_term(tantivy::Term::from_field_text(f_post_id, post.id.as_str()));
                writer.add_document(doc!(
                    f_post_id => post.id.as_str(),
                    f_title => post.title.as_str(),
                    f_content => post.content.as_str(),
                )).map_err(|e| AppError::Internal(anyhow::anyhow!("add: {e}")))?;
            }
            writer.commit().map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        }).await.map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }

    async fn delete_post(&self, post_id: &str) -> AppResult<()> {
        let index = self.index.clone();
        let f_post_id = self.fields.post_id;
        let post_id = post_id.to_string();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut writer = index.writer::<tantivy::TantivyDocument>(15_000_000)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("writer: {e}")))?;
            writer.delete_term(tantivy::Term::from_field_text(f_post_id, &post_id));
            writer.commit().map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        }).await.map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }

    async fn rebuild_all(&self, posts: &[SearchablePost]) -> AppResult<()> {
        let index = self.index.clone();
        let f_post_id = self.fields.post_id;
        let f_title = self.fields.title;
        let f_content = self.fields.content;
        let posts: Vec<SearchablePost> = posts.to_vec();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut writer = index.writer::<tantivy::TantivyDocument>(15_000_000)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("writer: {e}")))?;
            writer.delete_all_documents()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("delete all: {e}")))?;
            for post in &posts {
                writer.add_document(doc!(
                    f_post_id => post.id.as_str(),
                    f_title => post.title.as_str(),
                    f_content => post.content.as_str(),
                )).map_err(|e| AppError::Internal(anyhow::anyhow!("add: {e}")))?;
            }
            writer.commit().map_err(|e| AppError::Internal(anyhow::anyhow!("commit: {e}")))?;
            Ok(())
        }).await.map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
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
            let reader = index.reader()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("reader: {e}")))?;
            let searcher = reader.searcher();

            let qp = tantivy::query::QueryParser::for_index(&index, vec![f_title, f_content]);
            let parsed = qp.parse_query(&query_str)
                .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;

            let count = searcher.search(&*parsed, &Count)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("count: {e}")))? as i64;

            let offset = ((page - 1).max(0) * page_size) as usize;
            let limit = page_size as usize;

            let top_docs: Vec<(tantivy::Score, tantivy::DocAddress)> = searcher
                .search(&parsed, &tantivy::collector::TopDocs::with_limit(limit).and_offset(offset).order_by_score())
                .map_err(|e| AppError::Internal(anyhow::anyhow!("search: {e}")))?;

            let mut results = Vec::with_capacity(top_docs.len());
            for (score, addr) in top_docs {
                let doc: tantivy::TantivyDocument = searcher.doc(addr)
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("doc: {e}")))?;

                let post_id = doc.get_first(f_post_id)
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                let title_text = doc.get_first(f_title)
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                let content_text = doc.get_first(f_content)
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();

                let title_highlight = Some(Self::highlight(&query_str, &title_text));
                let excerpt = Self::make_excerpt(&content_text, &query_str, 200);
                let excerpt_highlight = excerpt.map(|e| Self::highlight(&query_str, &e));

                results.push(SearchResult { post_id, score, title_highlight, excerpt_highlight });
            }

            Ok((results, count))
        }).await.map_err(|e| AppError::Internal(anyhow::anyhow!("spawn: {e}")))?
    }
}

impl TantivyEngine {
    fn highlight(query: &str, text: &str) -> String {
        let mut result = text.to_string();
        for word in query.split_whitespace() {
            let lw = word.to_lowercase();
            let lt = result.to_lowercase();
            let mut buf = String::with_capacity(result.len() + 20);
            let mut last = 0;
            for (i, _) in lt.match_indices(&lw) {
                buf.push_str(&result[last..i]);
                buf.push_str("<em>");
                buf.push_str(&result[i..i + word.len()]);
                buf.push_str("</em>");
                last = i + word.len();
            }
            buf.push_str(&result[last..]);
            result = buf;
        }
        result
    }

    fn make_excerpt(content: &str, query: &str, max_len: usize) -> Option<String> {
        let first = query.split_whitespace().next()?;
        let pos = content.to_lowercase().find(&first.to_lowercase())?;
        let char_start = content[..pos].chars().count();
        let start_char = char_start.saturating_sub(max_len / 3);
        let end_char = (start_char + max_len).min(content.chars().count());
        let start = content.char_indices().nth(start_char).map_or(content.len(), |(i, _)| i);
        let end = content.char_indices().nth(end_char).map_or(content.len(), |(i, _)| i);
        let mut s = content[start..end].to_string();
        if start_char > 0 { s = format!("...{s}"); }
        if end_char < content.chars().count() { s = format!("{s}..."); }
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(id: &str, title: &str, content: &str) -> SearchablePost {
        SearchablePost { id: id.into(), title: title.into(), content: content.into() }
    }

    #[tokio::test]
    async fn index_and_search() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Rust编程入门", "Rust是一门安全的系统编程语言")).await.unwrap();
        e.index_post(&sp("p2", "Go语言教程", "Go是Google开发的编程语言")).await.unwrap();
        let (r, t) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn search_chinese() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "深入理解Rust", "Rust的所有权系统是其核心特性")).await.unwrap();
        e.index_post(&sp("p2", "Rust实战", "通过实战项目学习Rust")).await.unwrap();
        let (r, t) = e.search("所有权", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn batch_index() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_posts(&[sp("p1","标题一","内容一"), sp("p2","标题二","内容二"), sp("p3","标题三","内容三")]).await.unwrap();
        let (_r, t) = e.search("标题", 1, 10).await.unwrap();
        assert_eq!(t, 3);
    }

    #[tokio::test]
    async fn delete_post() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1","Rust入门","Rust编程")).await.unwrap();
        e.index_post(&sp("p2","Go入门","Go编程")).await.unwrap();
        e.delete_post("p1").await.unwrap();
        let (r, t) = e.search("入门", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p2");
    }

    #[tokio::test]
    async fn rebuild_all() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1","旧文章","旧内容")).await.unwrap();
        e.rebuild_all(&[sp("p2","新文章A","新内容A"), sp("p3","新文章B","新内容B")]).await.unwrap();
        assert_eq!(e.search("旧", 1, 10).await.unwrap().1, 0);
        assert_eq!(e.search("新文章", 1, 10).await.unwrap().1, 2);
    }

    #[tokio::test]
    async fn pagination() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..5).map(|i| sp(&format!("p{i}"), &format!("测试文章{i}"), &format!("内容{i}"))).collect();
        e.index_posts(&posts).await.unwrap();
        assert_eq!(e.search("测试", 1, 2).await.unwrap().1, 5);
        assert_eq!(e.search("测试", 1, 2).await.unwrap().0.len(), 2);
        assert_eq!(e.search("测试", 3, 2).await.unwrap().0.len(), 1);
    }

    #[tokio::test]
    async fn update_existing() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1","原标题","原内容")).await.unwrap();
        e.index_post(&sp("p1","新标题","新内容")).await.unwrap();
        assert_eq!(e.search("原标题", 1, 10).await.unwrap().1, 0);
        assert_eq!(e.search("新标题", 1, 10).await.unwrap().1, 1);
    }

    #[tokio::test]
    async fn no_results() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1","Rust入门","Rust编程")).await.unwrap();
        let (r, t) = e.search("不存在xyz", 1, 10).await.unwrap();
        assert_eq!(t, 0);
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn highlight_in_result() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1","Rust编程语言","Rust是一门现代系统编程语言")).await.unwrap();
        let (r, _) = e.search("Rust", 1, 10).await.unwrap();
        assert!(r[0].title_highlight.as_ref().unwrap().contains("<em>Rust</em>"));
    }

    #[test]
    fn highlight_unit() {
        assert_eq!(TantivyEngine::highlight("Rust", "学习Rust编程"), "学习<em>Rust</em>编程");
    }

    #[test]
    fn highlight_multi_word() {
        let result = TantivyEngine::highlight("Rust 语言", "Rust语言教程和Rust实践");
        assert!(result.contains("<em>Rust</em>"));
        assert!(result.contains("<em>语言</em>"));
    }

    #[test]
    fn highlight_case_insensitive() {
        let result = TantivyEngine::highlight("rust", "学习RUST编程");
        assert!(result.contains("<em>"));
    }

    #[test]
    fn highlight_no_match() {
        let text = "这是一段普通文本";
        assert_eq!(TantivyEngine::highlight("xyz", text), text);
    }

    #[test]
    fn excerpt_unit() {
        let c = "这是一段很长的内容，其中包含了关键词Rust，后面的内容应该被截断。";
        let e = TantivyEngine::make_excerpt(c, "Rust", 50);
        assert!(e.unwrap().contains("Rust"));
    }

    #[test]
    fn excerpt_short_content() {
        let c = "短内容Rust测试";
        let e = TantivyEngine::make_excerpt(c, "Rust", 200);
        let result = e.unwrap();
        assert!(result.contains("Rust"));
        assert!(!result.starts_with("..."));
    }

    #[test]
    fn excerpt_keyword_at_start() {
        let c = "Rust位于开头的测试内容后面还有很多文字填充";
        let e = TantivyEngine::make_excerpt(c, "Rust", 20);
        assert!(e.unwrap().contains("Rust"));
    }

    #[test]
    fn excerpt_keyword_at_end() {
        let c = "这是一段前面的内容最后才是关键词Rust";
        let e = TantivyEngine::make_excerpt(c, "Rust", 20);
        assert!(e.unwrap().contains("Rust"));
    }

    #[test]
    fn excerpt_no_match_returns_none() {
        let c = "没有关键词的文本";
        assert!(TantivyEngine::make_excerpt(c, "xyz", 50).is_none());
    }

    #[test]
    fn excerpt_multi_word_query_uses_first() {
        let c = "内容中包含First关键词和Second关键词";
        let e = TantivyEngine::make_excerpt(c, "First Second", 50);
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
        e.index_post(&sp("p1", "文章", "内容")).await.unwrap();
        e.rebuild_all(&[]).await.unwrap();
        assert_eq!(e.search("文章", 1, 10).await.unwrap().1, 0);
    }

    #[tokio::test]
    async fn delete_nonexistent_post_ok() {
        let e = TantivyEngine::open_in_memory().unwrap();
        assert!(e.delete_post("nonexistent").await.is_ok());
    }

    #[tokio::test]
    async fn search_returns_score() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Rust programming", "Rust is safe")).await.unwrap();
        let (r, _) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].score > 0.0);
    }

    #[tokio::test]
    async fn search_result_has_highlights() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "Rust编程语言", "Rust是一门现代系统编程语言")).await.unwrap();
        let (r, _) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].title_highlight.is_some());
        assert!(r[0].excerpt_highlight.is_some());
    }

    #[tokio::test]
    async fn search_result_post_id_matches() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("abc-123", "标题", "内容包含关键字match")).await.unwrap();
        let (r, _) = e.search("match", 1, 10).await.unwrap();
        assert_eq!(r[0].post_id, "abc-123");
    }

    #[tokio::test]
    async fn search_page_zero_treated_as_page_one() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..5).map(|i| sp(&format!("p{i}"), &format!("Rust{i}"), &format!("内容{i}"))).collect();
        e.index_posts(&posts).await.unwrap();
        let (r, t) = e.search("Rust", 0, 2).await.unwrap();
        assert_eq!(t, 5);
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn search_negative_page_treated_as_page_one() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..5).map(|i| sp(&format!("p{i}"), &format!("Rust{i}"), &format!("内容{i}"))).collect();
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
        e.index_post(&sp("p1", "Rust编程入门指南", "从零开始学习Rust编程语言")).await.unwrap();
        e.index_post(&sp("p2", "Python数据分析", "使用Python进行数据分析")).await.unwrap();
        let (r, t) = e.search("Rust", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn search_content_field() {
        let e = TantivyEngine::open_in_memory().unwrap();
        e.index_post(&sp("p1", "普通标题", "这篇内容包含特别的关键字Titanic")).await.unwrap();
        let (r, t) = e.search("Titanic", 1, 10).await.unwrap();
        assert_eq!(t, 1);
        assert_eq!(r[0].post_id, "p1");
    }

    #[tokio::test]
    async fn index_large_batch() {
        let e = TantivyEngine::open_in_memory().unwrap();
        let posts: Vec<SearchablePost> = (0..100)
            .map(|i| sp(&format!("p{i}"), &format!("批量文章{i}"), &format!("批量内容{i}")))
            .collect();
        e.index_posts(&posts).await.unwrap();
        let (_, t) = e.search("批量", 1, 10).await.unwrap();
        assert_eq!(t, 100);
    }
}

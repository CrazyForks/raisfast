//! Full-text search engine abstraction layer
//!
//! Provides the `SearchEngine` trait with two implementations: Tantivy (production) and Noop (fallback).
//! Controlled via the `search-tantivy` feature flag.

mod noop;
pub use noop::NoopSearchEngine;

#[cfg(feature = "search-tantivy")]
mod tantivy;
#[cfg(feature = "search-tantivy")]
pub use self::tantivy::TantivyEngine;

use crate::errors::app_error::AppResult;

/// A single search result entry
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub post_id: String,
    pub score: f32,
    pub title_highlight: Option<String>,
    pub excerpt_highlight: Option<String>,
}

/// Indexable post data (flat structure extracted from DB)
#[derive(Debug, Clone)]
pub struct SearchablePost {
    pub id: String,
    pub title: String,
    pub content: String,
}

/// Search engine interface
#[async_trait::async_trait]
pub trait SearchEngine: Send + Sync {
    /// Index a single post (updates if exists)
    async fn index_post(&self, post: &SearchablePost) -> AppResult<()>;

    /// Batch index multiple posts
    async fn index_posts(&self, posts: &[SearchablePost]) -> AppResult<()>;

    /// Delete a post from the index
    async fn delete_post(&self, post_id: &str) -> AppResult<()>;

    /// Clear and rebuild the entire index
    async fn rebuild_all(&self, posts: &[SearchablePost]) -> AppResult<()>;

    /// Search posts, returning results and total count
    async fn search(
        &self,
        query: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<SearchResult>, i64)>;

    /// Whether this is a no-op implementation (used to decide SQL LIKE fallback)
    fn is_noop(&self) -> bool {
        false
    }

    /// Human-readable engine name for health reporting
    fn engine_name(&self) -> &str {
        "unknown"
    }
}

/// Wrap search keywords in `<em>` tags within text, case-insensitive.
pub fn highlight_text(query: &str, text: &str) -> String {
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

/// Extract an excerpt from content containing the first matching keyword.
pub fn make_excerpt(content: &str, query: &str, max_len: usize) -> Option<String> {
    let first = query.split_whitespace().next()?;
    let pos = content.to_lowercase().find(&first.to_lowercase())?;
    let char_start = content[..pos].chars().count();
    let start_char = char_start.saturating_sub(max_len / 3);
    let end_char = (start_char + max_len).min(content.chars().count());
    let start = content
        .char_indices()
        .nth(start_char)
        .map_or(content.len(), |(i, _)| i);
    let end = content
        .char_indices()
        .nth(end_char)
        .map_or(content.len(), |(i, _)| i);
    let mut excerpt = content[start..end].to_string();
    if start > 0 {
        excerpt = format!("...{excerpt}");
    }
    if end < content.len() {
        excerpt = format!("{excerpt}...");
    }
    Some(excerpt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_single_word() {
        let result = highlight_text("rust", "Learning Rust is fun");
        assert_eq!(result, "Learning <em>Rust</em> is fun");
    }

    #[test]
    fn highlight_case_insensitive() {
        let result = highlight_text("hello", "Hello World, HELLO again");
        assert_eq!(result, "<em>Hello</em> World, <em>HELLO</em> again");
    }

    #[test]
    fn highlight_multiple_words() {
        let result = highlight_text("rust blog", "My Rust Blog");
        assert_eq!(result, "My <em>Rust</em> <em>Blog</em>");
    }

    #[test]
    fn highlight_no_match() {
        let result = highlight_text("python", "Learning Rust is fun");
        assert_eq!(result, "Learning Rust is fun");
    }

    #[test]
    fn make_excerpt_finds_keyword() {
        let content = "aaa bbb ccc ddd eee fff ggg hhh iii jjj";
        let result = make_excerpt(content, "eee", 20);
        assert!(result.is_some());
        let excerpt = result.unwrap();
        assert!(excerpt.contains("eee"));
    }

    #[test]
    fn make_excerpt_no_match() {
        let result = make_excerpt("hello world", "zzz", 20);
        assert!(result.is_none());
    }
}

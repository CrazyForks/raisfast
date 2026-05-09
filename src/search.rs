//! 全文搜索引擎抽象层
//!
//! 提供 `SearchEngine` trait，支持 Tantivy（生产）和 Noop（降级）两种实现。
//! 通过 `search-tantivy` feature flag 控制。

mod noop;
pub use noop::NoopSearchEngine;

#[cfg(feature = "search-tantivy")]
mod tantivy;
#[cfg(feature = "search-tantivy")]
pub use self::tantivy::TantivyEngine;

use crate::errors::app_error::AppResult;

/// 搜索结果条目
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub post_id: String,
    pub score: f32,
    pub title_highlight: Option<String>,
    pub excerpt_highlight: Option<String>,
}

/// 可索引的文章数据（从 DB 提取的扁平结构）
#[derive(Debug, Clone)]
pub struct SearchablePost {
    pub id: String,
    pub title: String,
    pub content: String,
}

/// 搜索引擎接口
#[async_trait::async_trait]
pub trait SearchEngine: Send + Sync {
    /// 索引单篇文章（存在则更新）
    async fn index_post(&self, post: &SearchablePost) -> AppResult<()>;

    /// 批量索引多篇文章
    async fn index_posts(&self, posts: &[SearchablePost]) -> AppResult<()>;

    /// 删除文章索引
    async fn delete_post(&self, post_id: &str) -> AppResult<()>;

    /// 清空并重建全部索引
    async fn rebuild_all(&self, posts: &[SearchablePost]) -> AppResult<()>;

    /// 搜索文章，返回结果和总数
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

/// 将搜索关键词在文本中用 `<em>` 标签包裹，大小写不敏感。
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

/// 从内容中截取包含首个匹配词的摘要片段。
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

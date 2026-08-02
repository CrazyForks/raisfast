//! No-op search engine (fallback when the feature is disabled)

use crate::errors::app_error::AppResult;

use super::{SearchEngine, SearchResult};

/// No-op search engine — all operations are no-ops, search always returns empty results
pub struct NoopSearchEngine;

#[async_trait::async_trait]
impl SearchEngine for NoopSearchEngine {
    fn engine_name(&self) -> &str {
        "noop"
    }

    async fn index_post(&self, _post: &super::SearchablePost) -> AppResult<()> {
        Ok(())
    }

    async fn index_posts(&self, _posts: &[super::SearchablePost]) -> AppResult<()> {
        Ok(())
    }

    async fn delete_post(&self, _post_id: &str) -> AppResult<()> {
        Ok(())
    }

    async fn rebuild_all(&self, _posts: &[super::SearchablePost]) -> AppResult<()> {
        Ok(())
    }

    async fn search(
        &self,
        _query: &str,
        _page: i64,
        _page_size: i64,
    ) -> AppResult<(Vec<SearchResult>, i64)> {
        Ok((vec![], 0))
    }

    fn is_noop(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> super::super::SearchablePost {
        super::super::SearchablePost {
            id: "p1".into(),
            title: "test".into(),
            content: "body".into(),
        }
    }

    #[tokio::test]
    async fn index_post_ok() {
        assert!(NoopSearchEngine.index_post(&sp()).await.is_ok());
    }

    #[tokio::test]
    async fn index_posts_ok() {
        assert!(NoopSearchEngine.index_posts(&[sp()]).await.is_ok());
    }

    #[tokio::test]
    async fn index_posts_empty_ok() {
        assert!(NoopSearchEngine.index_posts(&[]).await.is_ok());
    }

    #[tokio::test]
    async fn delete_post_ok() {
        assert!(NoopSearchEngine.delete_post("p1").await.is_ok());
    }

    #[tokio::test]
    async fn rebuild_all_ok() {
        assert!(NoopSearchEngine.rebuild_all(&[sp()]).await.is_ok());
    }

    #[tokio::test]
    async fn rebuild_all_empty_ok() {
        assert!(NoopSearchEngine.rebuild_all(&[]).await.is_ok());
    }

    #[tokio::test]
    async fn search_returns_empty() {
        let (results, total) = NoopSearchEngine.search("anything", 1, 10).await.unwrap();
        assert!(results.is_empty());
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn search_empty_query_returns_empty() {
        let (results, total) = NoopSearchEngine.search("", 1, 10).await.unwrap();
        assert!(results.is_empty());
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn search_large_page_returns_empty() {
        let (results, total) = NoopSearchEngine.search("test", 9999, 100).await.unwrap();
        assert!(results.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn is_noop_returns_true() {
        assert!(NoopSearchEngine.is_noop());
    }
}

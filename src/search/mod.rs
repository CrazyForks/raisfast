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

    /// 是否为空实现（用于判断是否应回退到 SQL LIKE 查询）
    fn is_noop(&self) -> bool {
        false
    }
}

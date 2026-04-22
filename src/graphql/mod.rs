//! GraphQL API 模块
//!
//! 为所有已注册的 Content Type 自动提供 GraphQL 接口。
//!
//! # 端点
//!
//! - `POST /api/v1/graphql` — 执行查询/变更
//! - `GET /api/v1/graphql` — GraphiQL IDE
//!
//! # 示例
//!
//! ```graphql
//! query {
//!   content(type: "post", page: 1, pageSize: 10, sort: "-created_at") {
//!     items { id data }
//!     total
//!     page
//!     pageSize
//!   }
//! }
//! ```

pub mod handler;
pub mod mutation;
pub mod query;
pub mod types;

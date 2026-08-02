//! GraphQL API module
//!
//! Automatically provides GraphQL interfaces for all registered Content Types.
//!
//! # Endpoints
//!
//! - `POST /api/v1/graphql` — Execute queries/mutations
//! - `GET /api/v1/graphql` — GraphiQL IDE
//!
//! # Example
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

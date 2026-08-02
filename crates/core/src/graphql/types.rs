//! GraphQL type definitions

use async_graphql::scalar;
use serde::{Deserialize, Serialize};

/// Custom JSON scalar type
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonScalar(pub serde_json::Value);

scalar!(JsonScalar, "JSON", "Arbitrary JSON value");

/// Content item
#[derive(async_graphql::SimpleObject, Clone)]
pub struct ContentItem {
    pub id: String,
    pub data: JsonScalar,
}

/// Paginated connection
#[derive(async_graphql::SimpleObject)]
pub struct ContentConnection {
    pub items: Vec<ContentItem>,
    pub total: Option<i32>,
    pub page: i32,
    pub page_size: i32,
}

/// Delete result
#[derive(async_graphql::SimpleObject)]
pub struct DeleteResult {
    pub success: bool,
    pub id: String,
}

pub struct QueryRoot;

pub struct MutationRoot;

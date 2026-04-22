//! GraphQL 类型定义

use async_graphql::scalar;
use serde::{Deserialize, Serialize};

/// 自定义 JSON 标量类型
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonScalar(pub serde_json::Value);

scalar!(JsonScalar, "JSON", "Arbitrary JSON value");

/// 内容条目
#[derive(async_graphql::SimpleObject, Clone)]
pub struct ContentItem {
    pub id: String,
    pub data: JsonScalar,
}

/// 分页连接
#[derive(async_graphql::SimpleObject)]
pub struct ContentConnection {
    pub items: Vec<ContentItem>,
    pub total: Option<i32>,
    pub page: i32,
    pub page_size: i32,
}

/// 删除结果
#[derive(async_graphql::SimpleObject)]
pub struct DeleteResult {
    pub success: bool,
    pub id: String,
}

pub struct QueryRoot;

pub struct MutationRoot;

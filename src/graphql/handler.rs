//! GraphQL Axum 路由处理器

use crate::AppState;
use crate::graphql::types::{MutationRoot, QueryRoot};
use crate::middleware::auth::OptionalAuth;
use async_graphql::EmptySubscription;
use async_graphql::http::GraphQLPlaygroundConfig;
use async_graphql::http::playground_source;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::response::Html;
use std::sync::Arc;

/// POST /api/v1/graphql
pub async fn graphql_handler(
    State(state): State<AppState>,
    OptionalAuth(auth): OptionalAuth,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let state_arc: Arc<AppState> = Arc::new(state);
    let schema = async_graphql::Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(state_arc)
        .data(auth)
        .finish();
    schema.execute(req.into_inner()).await.into()
}

/// GET /api/v1/graphql — GraphiQL IDE
pub async fn graphiql_handler() -> Html<String> {
    Html(playground_source(GraphQLPlaygroundConfig::new(
        "/api/v1/graphql",
    )))
}

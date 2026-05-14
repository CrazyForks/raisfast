//! GraphQL Axum route handlers

use crate::AppState;
use crate::graphql::types::{MutationRoot, QueryRoot};
use crate::middleware::auth::AuthUser;
use async_graphql::EmptySubscription;
use async_graphql::http::GraphQLPlaygroundConfig;
use async_graphql::http::playground_source;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::response::Html;
use std::sync::Arc;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let _restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/graphql",
        get,
        graphiql_handler,
        "system public",
        "graphql"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/graphql",
        post,
        graphql_handler,
        "system public",
        "graphql"
    )
}

/// POST /api/v1/graphql
pub async fn graphql_handler(
    State(state): State<AppState>,
    auth: AuthUser,
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

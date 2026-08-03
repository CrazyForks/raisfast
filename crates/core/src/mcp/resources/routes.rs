//! Route listing resource: exposes all registered HTTP routes to AI clients.

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::mcp::McpContext;
use crate::mcp::jsonrpc::ErrorObject;
use crate::mcp::registry::{ResourceMeta, ResourceProvider};
use crate::mcp::truncate;

/// Provider for the `raisfast://routes` resource.
pub struct RouteProvider;

impl ResourceProvider for RouteProvider {
    fn list(&self, _ctx: &McpContext) -> Vec<ResourceMeta> {
        vec![ResourceMeta {
            uri: "raisfast://routes".to_string(),
            name: "Registered HTTP Routes".to_string(),
            description: "Every route registered on this raisfast instance (REST + dynamic CMS)."
                .to_string(),
            mime_type: "application/json".to_string(),
        }]
    }

    fn read<'a>(
        &'a self,
        ctx: &'a McpContext,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Option<Value>, ErrorObject>> {
        Box::pin(async move { Ok(read_routes(ctx, uri).await) })
    }
}

async fn read_routes(ctx: &McpContext, uri: &str) -> Option<Value> {
    if uri != "raisfast://routes" {
        return None;
    }

    let routes: Vec<Value> = ctx
        .state
        .route_registry
        .iter()
        .map(|r| {
            json!({
                "method": r.method,
                "path": r.path,
                "source": r.source,
                "permission": r.permission,
            })
        })
        .collect();

    Some(truncate(json!(routes), ctx.config.max_result_chars))
}

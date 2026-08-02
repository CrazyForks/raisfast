//! MCP transport handlers — axum HTTP (Streamable HTTP) and stdio.
//!
//! The HTTP transport is mounted at `/api/v1/mcp` by `server::build_app`. It
//! accepts `POST` requests carrying a single JSON-RPC message and responds with
//! either a direct JSON-RPC response or an SSE stream. The stdio transport is
//! driven by the `raisfast mcp serve` CLI subcommand and reads/writes
//! newline-delimited JSON-RPC on stdin/stdout.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::{IntoResponse, Response, sse::Event as SseEvent, sse::Sse};
use futures::stream::{self, Stream};

use super::McpContext;
use super::jsonrpc::{self, ErrorObject, Id, Request};
use crate::AppState;

/// Register the MCP HTTP routes onto a router.
pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<AppState> {
    let _ = config;
    reg_route!(
        axum::Router::new(),
        registry,
        true, // MCP ignores the restful/simple toggle
        "/mcp",
        post,
        handle_mcp_post,
        "system",
        "mcp",
        "authed"
    )
}

/// `POST /api/v1/mcp` — the Streamable HTTP endpoint.
///
/// Accepts a JSON-RPC request. If the client advertises `text/event-stream`
/// in `Accept`, the response is streamed as SSE; otherwise a plain JSON
/// response is returned. Notifications (no `id`) get `202 Accepted`.
pub async fn handle_mcp_post(
    State(state): State<AppState>,
    auth: crate::middleware::auth::AuthUser,
    req: axum::extract::Request,
) -> Response {
    use axum::body::to_bytes;

    // Reject the transport outright if MCP is disabled.
    if !state.config.builtins.mcp || !state.config.mcp.enabled {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "code": 40400,
                "message": "MCP server disabled (BUILTIN_MCP=false or MCP_ENABLED=false)",
                "data": null,
            })),
        )
            .into_response();
    }

    let accept = req
        .headers()
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();

    let body_bytes = match to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return json_response(jsonrpc::error(Id::Num(0), ErrorObject::parse_error()))
                .into_response();
        }
    };

    let raw = match std::str::from_utf8(&body_bytes) {
        Ok(s) => s,
        Err(_) => {
            return json_response(jsonrpc::error(Id::Num(0), ErrorObject::parse_error()))
                .into_response();
        }
    };

    let request = match jsonrpc::parse(raw) {
        Ok(r) => r,
        Err(err) => {
            return json_response(jsonrpc::error(Id::Num(0), err)).into_response();
        }
    };

    // Notifications have no id → acknowledge without a body.
    let id = match request.id.clone() {
        Some(id) => id,
        None => return axum::http::StatusCode::ACCEPTED.into_response(),
    };

    let ctx = McpContext::from_state(std::sync::Arc::new(state), auth);
    let response = dispatch(&ctx, &request).await;

    // SSE when the client asked for it; otherwise plain JSON.
    if accept.contains("text/event-stream") {
        sse_response(id, response).into_response()
    } else {
        json_response(response).into_response()
    }
}

/// Run the MCP server over stdio (newline-delimited JSON-RPC).
///
/// Used by the `raisfast mcp serve` CLI subcommand. Blocks until stdin closes
/// or the process is terminated.
pub async fn serve_stdio(state: AppState) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let ctx = McpContext::for_stdio(std::sync::Arc::new(state));
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    tracing::info!(
        "MCP stdio server ready (local_user={:?})",
        ctx.config.local_user_id
    );

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let request = match jsonrpc::parse(&line) {
            Ok(r) => r,
            Err(err) => {
                let resp = jsonrpc::error(Id::Num(0), err);
                let mut s = serde_json::to_string(&resp).unwrap_or_default();
                s.push('\n');
                let _ = stdout.write_all(s.as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
        };

        // Notifications (no id) get no reply on stdio.
        if request.id.is_none() {
            let _ = dispatch(&ctx, &request).await;
            continue;
        }

        let response = dispatch(&ctx, &request).await;
        let mut s = serde_json::to_string(&response).unwrap_or_default();
        s.push('\n');
        let _ = stdout.write_all(s.as_bytes()).await;
        let _ = stdout.flush().await;
    }

    tracing::info!("MCP stdio server: stdin closed, exiting");
    Ok(())
}

/// Core JSON-RPC method dispatch, shared by both transports.
///
/// Takes a parsed [`Request`] and returns the full JSON-RPC response object
/// (success or error). MCP capabilities handled here: `initialize`,
/// `tools/list`, `tools/call`, `resources/list`, `resources/read`,
/// `prompts/list`, and `ping`.
async fn dispatch(ctx: &McpContext, request: &Request) -> serde_json::Value {
    let Some(id) = request.id.clone() else {
        // Notification: fire-and-forget. MCP only defines `notifications/initialized`.
        return serde_json::Value::Null;
    };

    let result: Result<serde_json::Value, ErrorObject> = match request.method.as_str() {
        "initialize" => Ok(initialize(ctx)),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(super::tools::list_tools(ctx)),
        "tools/call" => {
            let name = request
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = request.params.get("arguments").cloned().unwrap_or_default();
            super::tools::call_tool(ctx, name, &arguments).await
        }
        "resources/list" => Ok(super::resources::list_resources(ctx)),
        "resources/read" => {
            let uri = request
                .params
                .get("uri")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            super::resources::read_resource(ctx, uri).await
        }
        "prompts/list" => Ok(serde_json::json!({ "prompts": [] })),
        "resources/templates/list" => Ok(serde_json::json!({ "resourceTemplates": [] })),
        other => Err(ErrorObject::method_not_found(other)),
    };

    match result {
        Ok(value) => jsonrpc::success(id, value),
        Err(err) => jsonrpc::error(id, err),
    }
}

/// `initialize` response — advertises server info and supported capabilities.
fn initialize(_ctx: &McpContext) -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "listChanged": false, "subscribe": false },
            "prompts": { "listChanged": false }
        },
        "serverInfo": {
            "name": "raisfast",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

fn json_response(value: serde_json::Value) -> axum::Json<serde_json::Value> {
    axum::Json(value)
}

fn sse_response(
    _id: Id,
    value: serde_json::Value,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let event = SseEvent::default().event("message").data(value.to_string());
    Sse::new(stream::iter(vec![Ok(event)])).keep_alive(
        axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(15)),
    )
}

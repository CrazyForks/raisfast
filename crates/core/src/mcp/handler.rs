//! MCP transport handlers — axum HTTP (Streamable HTTP) and stdio.
//!
//! Implements the modern (2026-07-28) stateless protocol: no `initialize`
//! handshake, no sessions. Every request carries its protocol version and
//! client capabilities in `_meta`; the server validates them per-request.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::{IntoResponse, Response, sse::Event as SseEvent, sse::Sse};
use futures::stream::{self, Stream};

use super::McpContext;
use super::jsonrpc::{self, ErrorObject, Id, PROTOCOL_VERSION, Request};
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

/// `POST /api/v1/mcp` — the Streamable HTTP endpoint (2026-07-28).
///
/// Each request is a standalone JSON-RPC message. The server validates
/// `MCP-Protocol-Version` header + `_meta` metadata, then dispatches.
/// Notifications (no `id`) get `202 Accepted`.
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

    // Explicit auth check — the global permission_guard has a stale route_perms
    // bug (state cloned before permission map is populated), so "authed" routes
    // are not enforced at the middleware layer. We enforce here instead.
    if !auth.is_authenticated() {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "code": 40100,
                "message": "Unauthorized — valid Bearer token required",
                "data": null,
            })),
        )
            .into_response();
    }

    let headers = req.headers().clone();
    let accept = headers
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

    // Validate protocol version — but NOT for discovery methods (initialize /
    // server/discover), which are the version-negotiation mechanism itself.
    // Skipping them lets dual-era clients (Inspector, Claude Desktop) probe
    // the server without knowing the version upfront.
    let is_discovery = matches!(request.method.as_str(), "initialize" | "server/discover");
    if !is_discovery && let Some(err) = validate_protocol_version(&headers, &request) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            json_response(jsonrpc::error(
                request.id.clone().unwrap_or(Id::Num(0)),
                err,
            )),
        )
            .into_response();
    }

    // Notifications have no id → 202 Accepted, no body.
    if request.id.is_none() {
        return axum::http::StatusCode::ACCEPTED.into_response();
    }

    let ctx = McpContext::from_state(std::sync::Arc::new(state), auth);
    let response = dispatch(&ctx, &request).await;

    // Prefer JSON when the client accepts it (Inspector / Claude Desktop both
    // send "Accept: application/json, text/event-stream"). Only use SSE when
    // the client requests SSE exclusively or doesn't accept JSON at all.
    let prefer_json = accept.contains("application/json");
    if !prefer_json && accept.contains("text/event-stream") {
        sse_response(response).into_response()
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
        "MCP stdio server ready (protocol={}, local_user={:?})",
        PROTOCOL_VERSION,
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

/// Validate protocol version: header vs `_meta` consistency, and support check.
///
/// Only rejects when a version is **explicitly stated** and it either:
/// (a) differs between header and `_meta`, or
/// (b) is not `2026-07-28`.
///
/// When no version is provided at all (legacy clients like Inspector pre-handshake),
/// the request is accepted — this lets dual-era clients discover the server.
fn validate_protocol_version(
    headers: &axum::http::HeaderMap,
    request: &Request,
) -> Option<ErrorObject> {
    let header_version = headers
        .get("MCP-Protocol-Version")
        .and_then(|v| v.to_str().ok());

    let meta_version = jsonrpc::extract_protocol_version(&request.params);

    // If both present, they must match.
    if let (Some(hv), Some(mv)) = (header_version, meta_version.as_deref())
        && hv != mv
    {
        return Some(ErrorObject::header_mismatch(format!(
            "MCP-Protocol-Version header '{hv}' does not match _meta version '{mv}'"
        )));
    }

    // Only reject if an explicit version was provided AND it's not ours.
    // Missing version = legacy client, accept and assume ours.
    let version = header_version.or(meta_version.as_deref());
    if let Some(v) = version
        && v != PROTOCOL_VERSION
    {
        return Some(ErrorObject::unsupported_protocol_version(v));
    }

    None
}

/// Core JSON-RPC method dispatch, shared by both transports.
///
/// Takes a parsed [`Request`] and returns the full JSON-RPC response object.
/// Modern protocol methods: `server/discover`, `tools/list`, `tools/call`,
/// `resources/list`, `resources/read`, `prompts/list`, `ping`.
async fn dispatch(ctx: &McpContext, request: &Request) -> serde_json::Value {
    let Some(id) = request.id.clone() else {
        // Notification: no response.
        return serde_json::Value::Null;
    };

    let tool_reg = super::build_tool_registry();
    let resource_reg = super::build_resource_registry();
    let prompt_reg = super::build_prompt_registry();

    let result: Result<serde_json::Value, ErrorObject> = match request.method.as_str() {
        // server/discover is the modern discovery method (MCP 2026-07-28).
        // initialize is the legacy handshake — still supported for client
        // compatibility (Inspector, Claude Desktop send it as a connection
        // probe even in modern mode). Both return the same capability info.
        "server/discover" | "initialize" => Ok(discover()),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(tool_reg.list()),
        "tools/call" => {
            let name = request
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = request.params.get("arguments").cloned().unwrap_or_default();
            tool_reg.call(ctx, name, &arguments).await
        }
        "resources/list" => Ok(resource_reg.list(ctx)),
        "resources/read" => {
            let uri = request
                .params
                .get("uri")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            resource_reg.read(ctx, uri).await
        }
        "prompts/list" => Ok(prompt_reg.list()),
        "prompts/get" => {
            let name = request
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = request.params.get("arguments").cloned().unwrap_or_default();
            prompt_reg.get(ctx, name, &arguments).await
        }
        "resources/templates/list" => Ok(serde_json::json!({
            "resourceTemplates": [],
            "ttlMs": 300000,
            "cacheScope": "private"
        })),
        "completion/complete" => Ok(handle_completion(ctx, &request.params)),
        other => Err(ErrorObject::method_not_found(other)),
    };

    match result {
        Ok(value) => jsonrpc::success(id, value),
        Err(err) => jsonrpc::error(id, err),
    }
}

/// `completion/complete` — suggest values for resource template arguments.
///
/// Currently supports completion for `raisfast://content-types/{key}` — when
/// the client is typing a content type key, we suggest matching singular /
/// plural names from the registry.
fn handle_completion(ctx: &McpContext, params: &serde_json::Value) -> serde_json::Value {
    let arg_name = params
        .get("argument")
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let arg_value = params
        .get("argument")
        .and_then(|a| a.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let ref_type = params
        .get("ref")
        .and_then(|r| r.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let values: Vec<String> = match ref_type {
        "ref/resource" => {
            let uri = params
                .get("ref")
                .and_then(|r| r.get("uri"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Only support completion for the content-types template.
            if uri.contains("content-types") && arg_name == "key" {
                ctx.state
                    .content_type_registry
                    .all()
                    .iter()
                    .flat_map(|ct| {
                        [ct.singular.clone(), ct.plural.clone(), ct.table.clone()].into_iter()
                    })
                    .filter(|s| s.starts_with(arg_value))
                    .take(100)
                    .collect()
            } else {
                Vec::new()
            }
        }
        "ref/prompt" => {
            // No prompts currently — return empty.
            Vec::new()
        }
        _ => Vec::new(),
    };

    serde_json::json!({
        "completion": {
            "values": values,
            "hasMore": false
        }
    })
}

/// `server/discover` / `initialize` response — advertises supported versions,
/// capabilities, and server identity.
///
/// Serves double duty: modern clients call `server/discover`, legacy/dual-era
/// clients call `initialize`. The response includes both modern fields
/// (`supportedVersions`) and legacy fields (`protocolVersion`, `serverInfo`)
/// so both eras are satisfied.
fn discover() -> serde_json::Value {
    serde_json::json!({
        // Modern fields (2026-07-28)
        "supportedVersions": [PROTOCOL_VERSION],
        "capabilities": {
            "tools": {},
            "resources": {},
            "prompts": {},
            "completions": {}
        },
        "instructions": "raisfast MCP server. Use tools/list to discover available operations, \
            resources/list for data, and resources/read to fetch content. \
            Read raisfast://content-type-schema-guide before creating content types.",
        // Legacy fields (for Inspector / Claude Desktop compatibility)
        "protocolVersion": PROTOCOL_VERSION,
        "serverInfo": {
            "name": "raisfast",
            "version": env!("CARGO_PKG_VERSION"),
        },
        // Modern _meta (2026-07-28)
        "_meta": {
            "io.modelcontextprotocol/serverInfo": {
                "name": "raisfast",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }
    })
}

fn json_response(value: serde_json::Value) -> axum::Json<serde_json::Value> {
    axum::Json(value)
}

fn sse_response(value: serde_json::Value) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let event = SseEvent::default().event("message").data(value.to_string());
    Sse::new(stream::iter(vec![Ok(event)])).keep_alive(
        axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(15)),
    )
}

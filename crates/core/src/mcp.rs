//! Model Context Protocol (MCP) server.
//!
//! Exposes raisfast's CMS data, content-type schemas, and admin operations to
//! AI assistants (Claude Desktop, Cursor, etc.) over the MCP JSON-RPC protocol.
//!
//! # Transports
//!
//! - **Streamable HTTP** — `POST /api/v1/mcp` (registered in `server::build_app`).
//!   Authenticated via the standard JWT / API-token middleware (`authed`).
//! - **stdio** — `raisfast mcp serve` subcommand, for local clients that spawn
//!   raisfast as a child process. Runs as the user configured in `MCP_LOCAL_USER_ID`.
//!
//! # Architecture
//!
//! ```text
//! Transport (handler.rs)  →  JSON-RPC dispatch  →  Registry  →  Tool / ResourceProvider
//!     (HTTP / stdio)            (jsonrpc.rs)       (registry.rs)    (tools/ resources/)
//! ```
//!
//! Tools and resources implement the [`registry::Tool`] and
//! [`registry::ResourceProvider`] traits respectively, organised by domain
//! under `tools/` and `resources/`. Adding a new capability is a matter of
//! writing one struct and registering it in [`build_tool_registry`] /
//! [`build_resource_registry`].

pub mod handler;
mod jsonrpc;
pub mod prompts;
mod registry;
pub mod resources;
pub mod tools;

use std::sync::Arc;

use serde_json::Value;

use crate::config::app::McpConfig;
use crate::content_type::repository::ContentRepository;
use crate::middleware::auth::AuthUser;
use crate::models::user::UserRole;

pub(crate) use registry::{PromptRegistry, ResourceRegistry, ToolRegistry};

/// Shared context handed to every tool / resource handler.
///
/// Holds clones of the bits of [`crate::AppState`] the MCP handlers need, plus
/// the resolved [`AuthUser`] used to authorize service-layer calls.
pub struct McpContext {
    pub state: Arc<crate::AppState>,
    pub auth: AuthUser,
    pub repo: ContentRepository,
    pub config: McpConfig,
}

impl McpContext {
    /// Build a context for an incoming HTTP request (auth resolved by middleware).
    pub(crate) fn from_state(state: Arc<crate::AppState>, auth: AuthUser) -> Self {
        let config = state.config.mcp.clone();
        let repo = ContentRepository::new(state.pool.clone());
        Self {
            state,
            auth,
            repo,
            config,
        }
    }

    /// Build a context for the stdio transport, impersonating the configured local user.
    pub(crate) fn for_stdio(state: Arc<crate::AppState>) -> Self {
        let cfg = state.config.mcp.clone();
        let auth = match cfg.local_user_id {
            Some(uid) => AuthUser::from_parts(
                Some(uid),
                UserRole::Admin,
                Some(cfg.local_tenant_id.clone()),
            ),
            None => AuthUser::from_parts(None, UserRole::Reader, Some(cfg.local_tenant_id.clone())),
        };
        let repo = ContentRepository::new(state.pool.clone());
        Self {
            state,
            auth,
            repo,
            config: cfg,
        }
    }
}

/// Truncate a JSON value's serialized form to at most `max_chars` bytes,
/// landing on a UTF-8 char boundary and appending an ellipsis marker.
///
/// Protects AI clients from oversized payloads. The returned `Value::String`
/// is **display text**, not re-parseable JSON — downstream code should treat it
/// as opaque text, never `json::parse` it.
pub(crate) fn truncate(value: Value, max_chars: usize) -> Value {
    let s = match serde_json::to_string(&value) {
        Ok(s) => s,
        Err(_) => return value,
    };
    if s.len() <= max_chars {
        return value;
    }
    let mut end = max_chars;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    Value::String(format!(
        "{} …[truncated: payload exceeded {max_chars} chars]",
        &s[..end]
    ))
}

/// Build the complete tool registry with all built-in tools registered.
///
/// To add a new tool: implement [`Tool`] on a struct in the relevant domain
/// module under `tools/`, then add one `reg.register(...)` line here.
pub(crate) fn build_tool_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    tools::register_all(&mut reg);
    reg
}

/// Build the complete resource registry with all built-in providers registered.
pub(crate) fn build_resource_registry() -> ResourceRegistry {
    let mut reg = ResourceRegistry::new();
    resources::register_all(&mut reg);
    reg
}

/// Build the complete prompt registry with all built-in providers registered.
pub(crate) fn build_prompt_registry() -> PromptRegistry {
    let mut reg = PromptRegistry::new();
    prompts::register_all(&mut reg);
    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncate_passthrough_when_small() {
        let v = json!({ "ok": true });
        let out = truncate(v.clone(), 1000);
        assert_eq!(out, v, "small values pass through unchanged");
    }

    #[test]
    fn truncate_produces_valid_utf8_string() {
        let v: Value = (0..200)
            .map(|i| format!("item-{i}-é-€"))
            .collect::<Vec<_>>()
            .into();
        let out = truncate(v, 100);
        let s = out.as_str().expect("truncated result is a string");
        assert!(s.contains("…[truncated"));
        assert!(std::str::from_utf8(s.as_bytes()).is_ok());
    }

    #[test]
    fn truncate_marker_has_no_literal_newline() {
        let v = json!(vec!["x"; 10_000]);
        let out = truncate(v, 50);
        let s = out.as_str().unwrap();
        assert!(
            !s.contains('\n'),
            "marker must not embed literal newline (breaks JSON consumers)"
        );
    }
}

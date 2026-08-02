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
//! # Design
//!
//! A manual JSON-RPC 2.0 implementation (no external `rmcp` dependency) to keep
//! the dependency tree clean and match the existing manual SSE / WS / GraphQL
//! handlers. Tools wrap the Service layer so policy checks, the event bus, and
//! audit logging are preserved — the model layer is never reached directly.
//!
//! Capabilities exposed:
//! - **Tools** — `list_content_types`, `list_entries`, `get_entry`, `create_entry`,
//!   `list_posts`, `get_post`, `create_post`
//! - **Resources** — `raisfast://content-types`, `raisfast://content-types/{key}`,
//!   `raisfast://routes`
//! - **Prompts** — `draft_post`, `summarize_posts`

pub mod handler;
mod jsonrpc;
pub mod resources;
pub mod tools;

use std::sync::Arc;

use serde_json::Value;

use crate::config::app::McpConfig;
use crate::content_type::repository::ContentRepository;
use crate::middleware::auth::AuthUser;
use crate::models::user::UserRole;

/// Shared context handed to every tool / resource handler.
///
/// Holds clones of the bits of [`crate::AppState`] the MCP handlers need, plus
/// the resolved [`AuthUser`] used to authorize service-layer calls.
pub(crate) struct McpContext {
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
    // Walk back to the nearest UTF-8 char boundary so we never split a
    // multi-byte sequence (which would panic `str` ops or produce invalid UTF-8).
    let mut end = max_chars;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    Value::String(format!(
        "{} …[truncated: payload exceeded {max_chars} chars]",
        &s[..end]
    ))
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
        // Build a value whose serialized form is well over 100 bytes, with
        // multi-byte UTF-8 chars near the cut point.
        let v: Value = (0..200)
            .map(|i| format!("item-{i}-é-€"))
            .collect::<Vec<_>>()
            .into();
        let out = truncate(v, 100);
        let s = out.as_str().expect("truncated result is a string");
        // Must be valid UTF-8 (no panic, no broken sequence).
        assert!(s.contains("…[truncated"));
        // Cut point must be on a char boundary (is_char_boundary already guarantees
        // this, but assert UTF-8 validity explicitly).
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

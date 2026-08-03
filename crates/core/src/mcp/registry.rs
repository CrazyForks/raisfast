//! Trait-based plugin registry for MCP tools and resources.
//!
//! Each tool is a struct implementing [`Tool`]; each resource group is a struct
//! implementing [`ResourceProvider`]. New capabilities are added by defining a
//! struct and registering it in [`crate::mcp::build_tool_registry`] /
//! [`crate::mcp::build_resource_registry`] — no monolithic match arms to touch.
//!
//! # Adding a new tool
//!
//! ```ignore
//! // src/mcp/tools/my_domain.rs
//! use crate::mcp::*;
//!
//! pub struct MyTool;
//!
//! impl_tool!(MyTool, "my_tool", "Does something useful", {
//!     "type": "object", "properties": {}
//! });
//!
//! impl MyTool {
//!     /// Actual tool logic — write it as a normal `async fn`.
//!     async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
//!         // implementation
//!         Ok(json!({}))
//!     }
//! }
//! ```
//!
//! Then register in `tools/mod.rs`.

use futures::future::BoxFuture;
use serde_json::Value;

use super::McpContext;
use super::jsonrpc::ErrorObject;

// ─── Tool trait ─────────────────────────────────────────────────────────

/// A single MCP tool — a parameterized operation callable by AI clients.
///
/// Implementors are typically zero-sized structs; all state comes from the
/// [`McpContext`] passed to [`call`].
pub trait Tool: Send + Sync + 'static {
    /// Unique tool name (e.g. `"create_post"`).
    fn name(&self) -> &str;

    /// Optional human-readable title for display in UIs (MCP 2026-07-28).
    /// Defaults to the tool name if not overridden.
    fn title(&self) -> Option<&str> {
        None
    }

    /// Human-readable description shown to AI clients in `tools/list`.
    fn description(&self) -> &str;

    /// JSON Schema describing the tool's input parameters.
    fn input_schema(&self) -> Value;

    /// JSON Schema describing the tool's **structured output** (MCP
    /// `outputSchema`). When `Some`, the dispatcher includes
    /// `structuredContent` alongside the text envelope, enabling clients to do
    /// type-safe parsing rather than scraping free text. Default: `None`.
    fn output_schema(&self) -> Option<Value> {
        None
    }

    /// Whether this tool should advertise `readOnlyHint` in its annotations.
    /// Read-only tools (no side effects) are tagged so clients know they can be
    /// retried safely. Default: `false`.
    fn read_only(&self) -> bool {
        false
    }

    /// Execute the tool. Returns a JSON value that will be wrapped in the MCP
    /// content envelope by the dispatcher.
    fn call<'a>(
        &'a self,
        ctx: &'a McpContext,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ErrorObject>>;
}

/// Declare `impl Tool` for a struct whose `async fn run(ctx, args)` holds the
/// actual logic.
///
/// Generates `name`, `description`, `input_schema` from literals, and a `call`
/// that boxes the future returned by `Self::run`. This avoids per-tool
/// `Box::pin(async move { ... })` boilerplate.
///
/// # Forms
///
/// Without title (uses tool name as title):
/// ```ignore
/// impl_tool!(MyTool, "my_tool", "Does something", { "type": "object" });
/// ```
///
/// With explicit title (MCP 2026-07-28 `title` field):
/// ```ignore
/// impl_tool!(MyTool, "my_tool", "My Tool", "Does something", { "type": "object" });
/// ```
///
/// `title()`, `output_schema()`, and `read_only()` default to `None`/`false`.
/// Override them in a separate manual impl if needed.
#[macro_export]
macro_rules! impl_tool {
    ($struct:ty, $name:literal, $title:literal, $desc:literal, $schema:tt) => {
        impl $crate::mcp::registry::Tool for $struct {
            fn name(&self) -> &str {
                $name
            }
            fn title(&self) -> Option<&str> {
                Some($title)
            }
            fn description(&self) -> &str {
                $desc
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!($schema)
            }
            fn call<'a>(
                &'a self,
                ctx: &'a $crate::mcp::McpContext,
                args: &'a serde_json::Value,
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<serde_json::Value, $crate::mcp::jsonrpc::ErrorObject>,
            > {
                Box::pin(async move { <$struct>::run(ctx, args).await })
            }
        }
    };
    ($struct:ty, $name:literal, $desc:literal, $schema:tt) => {
        impl $crate::mcp::registry::Tool for $struct {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!($schema)
            }
            fn call<'a>(
                &'a self,
                ctx: &'a $crate::mcp::McpContext,
                args: &'a serde_json::Value,
            ) -> ::futures::future::BoxFuture<
                'a,
                Result<serde_json::Value, $crate::mcp::jsonrpc::ErrorObject>,
            > {
                Box::pin(async move { <$struct>::run(ctx, args).await })
            }
        }
    };
}

// ─── ResourceProvider trait ─────────────────────────────────────────────

/// Metadata for a resource, returned by [`ResourceProvider::list`].
#[derive(Debug, Clone)]
pub struct ResourceMeta {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime_type: String,
}

/// A provider of one or more MCP resources (URI-addressable read-only data).
///
/// A single provider can serve multiple URIs (e.g. a content-types provider
/// serves `raisfast://content-types`, `raisfast://content-types/{key}`, and
/// `raisfast://content-type-schema-guide`).
pub trait ResourceProvider: Send + Sync + 'static {
    /// Return metadata for every resource this provider can serve.
    fn list(&self, ctx: &McpContext) -> Vec<ResourceMeta>;

    /// Read a resource by URI. Return `None` if this provider doesn't handle
    /// the given URI.
    fn read<'a>(
        &'a self,
        ctx: &'a McpContext,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Option<Value>, ErrorObject>>;
}

// ─── ToolRegistry ───────────────────────────────────────────────────────

/// Central registry of all MCP tools, indexed by name for O(1) dispatch.
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool. The tool's `name()` must be unique.
    pub fn register(&mut self, tool: impl Tool) -> &mut Self {
        tracing::debug!(tool = tool.name(), "registered MCP tool");
        self.tools.push(Box::new(tool));
        self
    }

    /// Number of registered tools.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Produce the `tools/list` response.
    pub fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .tools
            .iter()
            .map(|t| {
                let mut entry = serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "inputSchema": t.input_schema(),
                });
                if let Some(title) = t.title() {
                    entry["title"] = serde_json::Value::String(title.to_string());
                }
                if let Some(schema) = t.output_schema() {
                    entry["outputSchema"] = schema;
                }
                if t.read_only() {
                    entry["annotations"] = serde_json::json!({
                        "readOnlyHint": true
                    });
                }
                entry
            })
            .collect();
        serde_json::json!({
            "tools": tools,
            "ttlMs": 300000,
            "cacheScope": "private"
        })
    }

    /// Dispatch `tools/call` to the matching tool. Returns a method-not-found
    /// error if no tool with the given name is registered.
    pub async fn call(
        &self,
        ctx: &McpContext,
        name: &str,
        args: &Value,
    ) -> Result<Value, ErrorObject> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| ErrorObject::method_not_found(name))?;

        let result = tool.call(ctx, args).await?;

        // Build the MCP content envelope. When the tool declares an
        // output_schema, also include `structuredContent` so clients can parse
        // the result as typed JSON rather than scraping text.
        let mut response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": value_to_text(&result, ctx.config.max_result_chars)
            }]
        });
        if tool.output_schema().is_some() {
            response["structuredContent"] = result;
        }
        Ok(response)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── ResourceRegistry ───────────────────────────────────────────────────

/// Central registry of all MCP resource providers.
pub struct ResourceRegistry {
    providers: Vec<Box<dyn ResourceProvider>>,
}

impl ResourceRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a resource provider.
    pub fn register(&mut self, provider: impl ResourceProvider) -> &mut Self {
        tracing::debug!("registered MCP resource provider");
        self.providers.push(Box::new(provider));
        self
    }

    /// Produce the `resources/list` response (aggregated across all providers).
    pub fn list(&self, ctx: &McpContext) -> Value {
        let resources: Vec<Value> = self
            .providers
            .iter()
            .flat_map(|p| p.list(ctx))
            .map(|r| {
                serde_json::json!({
                    "uri": r.uri,
                    "name": r.name,
                    "description": r.description,
                    "mimeType": r.mime_type,
                })
            })
            .collect();
        serde_json::json!({
            "resources": resources,
            "ttlMs": 300000,
            "cacheScope": "private"
        })
    }

    /// Dispatch `resources/read` — tries each provider until one returns `Some`.
    pub async fn read(&self, ctx: &McpContext, uri: &str) -> Result<Value, ErrorObject> {
        for provider in &self.providers {
            if let Some(body) = provider.read(ctx, uri).await? {
                let text = if body.is_string() {
                    body.as_str().unwrap_or_default().to_string()
                } else {
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                };
                return Ok(serde_json::json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": text,
                    }]
                }));
            }
        }
        Err(ErrorObject::new(-32002, format!("unknown resource: {uri}")))
    }
}

impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Shared helpers ─────────────────────────────────────────────────────

/// Render a tool result value as display text (pretty JSON, safely truncated
/// to `max` bytes on a UTF-8 char boundary).
fn value_to_text(value: &Value, max: usize) -> String {
    let text = match value {
        Value::String(s) => return s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} …[truncated at {max} chars]", &text[..end])
}

// ─── PromptProvider trait + PromptRegistry ──────────────────────────────

/// Metadata for a prompt's argument.
#[derive(Debug, Clone)]
pub struct PromptArgMeta {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// Metadata for a prompt, returned by [`PromptProvider::list`].
#[derive(Debug, Clone)]
pub struct PromptMeta {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub arguments: Vec<PromptArgMeta>,
}

/// A provider of one or more MCP prompts (user-triggered slash commands).
///
/// Each provider owns a domain of prompts (e.g. blog prompts, CMS prompts).
/// `list` returns their catalogue; `get` resolves a named prompt into concrete
/// [`PromptMessage`]s, optionally filling in server-side context.
pub trait PromptProvider: Send + Sync + 'static {
    /// Return metadata for every prompt this provider exposes.
    fn list(&self) -> Vec<PromptMeta>;

    /// Resolve a prompt by name + arguments into concrete messages.
    /// Return `None` if this provider doesn't handle the given name.
    fn get<'a>(
        &'a self,
        ctx: &'a McpContext,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Option<Vec<PromptMessage>>, ErrorObject>>;
}

/// A single message in a resolved prompt (role + text content).
#[derive(Debug, Clone)]
pub struct PromptMessage {
    pub role: PromptRole,
    pub content: PromptContent,
}

#[derive(Debug, Clone, Copy)]
pub enum PromptRole {
    User,
    Assistant,
}

impl PromptRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromptRole::User => "user",
            PromptRole::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone)]
pub enum PromptContent {
    Text(String),
    ResourceLink { uri: String, name: String },
}

/// Central registry of all MCP prompt providers.
pub struct PromptRegistry {
    providers: Vec<Box<dyn PromptProvider>>,
}

impl PromptRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a prompt provider.
    pub fn register(&mut self, provider: impl PromptProvider) -> &mut Self {
        tracing::debug!("registered MCP prompt provider");
        self.providers.push(Box::new(provider));
        self
    }

    /// Produce the `prompts/list` response.
    pub fn list(&self) -> Value {
        let prompts: Vec<Value> = self
            .providers
            .iter()
            .flat_map(|p| p.list())
            .map(|p| {
                let mut entry = serde_json::json!({
                    "name": p.name,
                    "description": p.description,
                    "arguments": p.arguments.iter().map(|a| {
                        serde_json::json!({
                            "name": a.name,
                            "description": a.description,
                            "required": a.required,
                        })
                    }).collect::<Vec<_>>(),
                });
                if let Some(title) = p.title {
                    entry["title"] = Value::String(title);
                }
                entry
            })
            .collect();
        serde_json::json!({
            "prompts": prompts,
            "ttlMs": 300000,
            "cacheScope": "private"
        })
    }

    /// Resolve `prompts/get` — tries each provider until one returns `Some`.
    pub async fn get(
        &self,
        ctx: &McpContext,
        name: &str,
        args: &Value,
    ) -> Result<Value, ErrorObject> {
        for provider in &self.providers {
            if let Some(messages) = provider.get(ctx, name, args).await? {
                let msgs: Vec<Value> = messages
                    .iter()
                    .map(|m| match &m.content {
                        PromptContent::Text(text) => serde_json::json!({
                            "role": m.role.as_str(),
                            "content": { "type": "text", "text": text }
                        }),
                        PromptContent::ResourceLink { uri, name } => serde_json::json!({
                            "role": m.role.as_str(),
                            "content": {
                                "type": "resource_link",
                                "uri": uri,
                                "name": name,
                            }
                        }),
                    })
                    .collect();
                return Ok(serde_json::json!({ "messages": msgs }));
            }
        }
        Err(ErrorObject::invalid_params(format!(
            "unknown prompt: {name}"
        )))
    }
}

impl Default for PromptRegistry {
    fn default() -> Self {
        Self::new()
    }
}

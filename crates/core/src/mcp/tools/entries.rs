//! Dynamic content entry tools: list, get, create entries in any content type.

use serde_json::{Value, json};

use crate::mcp::jsonrpc::ErrorObject;
use crate::mcp::{McpContext, truncate};

/// Register all entry CRUD tools.
pub fn register(reg: &mut crate::mcp::registry::ToolRegistry) {
    reg.register(ListEntries);
    reg.register(GetEntry);
    reg.register(CreateEntry);
}

// ═════════════════════════════════════════════════════════════════════════
// list_entries
// ═════════════════════════════════════════════════════════════════════════

pub struct ListEntries;

crate::impl_tool!(
    ListEntries,
    "list_entries",
    "List entries of a dynamic content type by its plural key (e.g. \"topics\", \
    \"forum/topics\"). Returns a JSON array of records.",
    {
        "type": "object",
        "required": ["content_type"],
        "properties": {
            "content_type": { "type": "string", "description": "Plural key of the content type" },
            "limit": { "type": "integer", "description": "Max records (default 20, max 100)", "default": 20 }
        }
    }
);

impl ListEntries {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
        let plural = args
            .get("content_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorObject::invalid_params("missing 'content_type'"))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(100) as i64;

        let ct = ctx
            .state
            .content_type_registry
            .get_by_plural(plural)
            .or_else(|| ctx.state.content_type_registry.get(plural))
            .ok_or_else(|| {
                ErrorObject::invalid_params(format!("unknown content type: {plural}"))
            })?;

        let query = crate::content_type::repository::ContentQuery {
            page: 1,
            page_size: limit,
            max_page_size: ctx.state.config.rule_engine.cms_max_page_size as i64,
            tenant_id: ctx.auth.tenant_id().map(|s| s.to_string()),
            ..Default::default()
        };

        let (rows, total) = ctx
            .repo
            .find(&ct, query)
            .await
            .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

        Ok(truncate(
            json!({ "items": rows, "total": total, "content_type": plural }),
            ctx.config.max_result_chars,
        ))
    }
}

// ═════════════════════════════════════════════════════════════════════════
// get_entry
// ═════════════════════════════════════════════════════════════════════════

pub struct GetEntry;

crate::impl_tool!(
    GetEntry,
    "get_entry",
    "Fetch a single entry of a dynamic content type by its ID.",
    {
        "type": "object",
        "required": ["content_type", "id"],
        "properties": {
            "content_type": { "type": "string" },
            "id": { "type": "string", "description": "The entry's ID" }
        }
    }
);

impl GetEntry {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
        let plural = args
            .get("content_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorObject::invalid_params("missing 'content_type'"))?;
        let id_str = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorObject::invalid_params("missing 'id'"))?;

        let ct = ctx
            .state
            .content_type_registry
            .get_by_plural(plural)
            .or_else(|| ctx.state.content_type_registry.get(plural))
            .ok_or_else(|| {
                ErrorObject::invalid_params(format!("unknown content type: {plural}"))
            })?;

        let id = crate::types::snowflake_id::parse_id(id_str)
            .map_err(|e| ErrorObject::invalid_params(format!("invalid id: {e}")))?;

        let row = ctx
            .repo
            .find_by_id(&ct, id, ctx.auth.tenant_id(), true)
            .await
            .map_err(|e| ErrorObject::internal_error(e.to_string()))?
            .ok_or_else(|| ErrorObject::new(-32001, format!("entry {id_str} not found")))?;

        Ok(truncate(row, ctx.config.max_result_chars))
    }
}

// ═════════════════════════════════════════════════════════════════════════
// create_entry
// ═════════════════════════════════════════════════════════════════════════

pub struct CreateEntry;

crate::impl_tool!(
    CreateEntry,
    "create_entry",
    "Create a new entry in a dynamic content type. Requires an admin/authed MCP session \
    (HTTP API token or stdio local admin).",
    {
        "type": "object",
        "required": ["content_type", "fields"],
        "properties": {
            "content_type": { "type": "string" },
            "fields": { "type": "object", "description": "Field values keyed by field name" }
        }
    }
);

impl CreateEntry {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
        if ctx.auth.ensure_author().is_err() {
            return Err(ErrorObject::new(
                -32603,
                "create_entry requires an author/admin MCP session",
            ));
        }

        let plural = args
            .get("content_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorObject::invalid_params("missing 'content_type'"))?;
        let fields = args
            .get("fields")
            .ok_or_else(|| ErrorObject::invalid_params("missing 'fields'"))?
            .clone();

        let ct = ctx
            .state
            .content_type_registry
            .get_by_plural(plural)
            .or_else(|| ctx.state.content_type_registry.get(plural))
            .ok_or_else(|| {
                ErrorObject::invalid_params(format!("unknown content type: {plural}"))
            })?;

        let save_ctx = crate::content_type::repository::SaveContext::from_auth(&ctx.auth);
        let created = ctx
            .repo
            .create(&ct, fields, ctx.auth.tenant_id(), &save_ctx)
            .await
            .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

        Ok(truncate(created, ctx.config.max_result_chars))
    }
}

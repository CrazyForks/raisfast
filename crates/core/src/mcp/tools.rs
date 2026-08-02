//! MCP Tools — discrete, parameterized operations exposed to AI clients.
//!
//! Each tool wraps the raisfast Service layer (never the model layer directly),
//! so ownership/policy checks, the event bus, and audit logging are preserved.
//!
//! Tool definitions are produced by [`list_tools`] and dispatched by
//! [`call_tool`]. The set is intentionally coarse: one tool per business
//! operation rather than one per SQL statement, to minimize AI tool-call cost.

use serde_json::{Value, json};
use std::sync::LazyLock;

use super::McpContext;
use super::jsonrpc::ErrorObject;

/// One entry in the static tool catalogue.
struct ToolDef {
    name: &'static str,
    description: &'static str,
    /// JSON Schema describing the tool's input parameters.
    input_schema: Value,
}

/// `tools/list` — return the catalogue of tools this server exposes.
pub(crate) fn list_tools(_ctx: &McpContext) -> Value {
    let tools = ALL_TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect::<Vec<_>>();
    json!({ "tools": tools })
}

/// `tools/call` — dispatch to the named tool.
pub(crate) async fn call_tool(
    ctx: &McpContext,
    name: &str,
    arguments: &Value,
) -> Result<Value, ErrorObject> {
    let result = match name {
        "list_content_types" => list_content_types(ctx).await,
        "list_entries" => list_entries(ctx, arguments).await,
        "get_entry" => get_entry(ctx, arguments).await,
        "create_entry" => create_entry(ctx, arguments).await,
        "create_content_type" => create_content_type(ctx, arguments).await,
        "update_content_type" => update_content_type(ctx, arguments).await,
        "delete_content_type" => delete_content_type(ctx, arguments).await,
        "list_posts" => list_posts(ctx, arguments).await,
        "get_post" => get_post(ctx, arguments).await,
        "create_post" => create_post(ctx, arguments).await,
        _ => {
            return Err(ErrorObject::method_not_found(name));
        }
    };
    // Wrap the tool's JSON result in the MCP content envelope.
    result.map(|value| json!({ "content": [{ "type": "text", "text": value_to_text(&value, ctx.config.max_result_chars) }] }))
}

/// Render a tool result value as display text (pretty JSON, safely truncated to
/// `max` bytes on a UTF-8 char boundary).
fn value_to_text(value: &Value, max: usize) -> String {
    let text = match value {
        // truncate() already produced display text — pass through unchanged.
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

// ─── Tool catalogue ────────────────────────────────────────────────────

static ALL_TOOLS: LazyLock<Vec<ToolDef>> = LazyLock::new(|| {
    vec![
        ToolDef {
            name: "list_content_types",
            description: "List all dynamic CMS content types (defined by TOML schema files). \
                Returns each type's name, description, fields, and keys. Blog posts are NOT \
                included here — use the dedicated `list_posts`/`create_post` tools for those. \
                Call this first to discover what data the raisfast instance manages.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: "list_entries",
            description: "List entries of a dynamic content type by its plural key \
                (e.g. \"topics\", \"forum/topics\"). Returns a JSON array of records.",
            input_schema: json!({
                "type": "object",
                "required": ["content_type"],
                "properties": {
                    "content_type": { "type": "string", "description": "Plural key of the content type" },
                    "limit": { "type": "integer", "description": "Max records (default 20, max 100)", "default": 20 }
                }
            }),
        },
        ToolDef {
            name: "get_entry",
            description: "Fetch a single entry of a dynamic content type by its ID.",
            input_schema: json!({
                "type": "object",
                "required": ["content_type", "id"],
                "properties": {
                    "content_type": { "type": "string" },
                    "id": { "type": "string", "description": "The entry's ID" }
                }
            }),
        },
        ToolDef {
            name: "create_entry",
            description: "Create a new entry in a dynamic content type. Requires an admin/authed \
                MCP session (HTTP API token or stdio local admin).",
            input_schema: json!({
                "type": "object",
                "required": ["content_type", "fields"],
                "properties": {
                    "content_type": { "type": "string" },
                    "fields": { "type": "object", "description": "Field values keyed by field name" }
                }
            }),
        },
        ToolDef {
            name: "list_posts",
            description: "List blog posts (published). Supports keyword search and pagination.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "page": { "type": "integer", "default": 1 },
                    "page_size": { "type": "integer", "default": 10 },
                    "q": { "type": "string", "description": "Optional keyword search" }
                }
            }),
        },
        ToolDef {
            name: "get_post",
            description: "Fetch a single blog post by its slug.",
            input_schema: json!({
                "type": "object",
                "required": ["slug"],
                "properties": { "slug": { "type": "string" } }
            }),
        },
        ToolDef {
            name: "create_post",
            description: "Create a new blog post. Requires an author/admin MCP session.",
            input_schema: json!({
                "type": "object",
                "required": ["title", "content"],
                "properties": {
                    "title": { "type": "string" },
                    "content": { "type": "string", "description": "Markdown body" },
                    "slug": { "type": "string" },
                    "excerpt": { "type": "string" },
                    "status": { "type": "string", "enum": ["draft", "published", "scheduled"], "default": "draft" }
                }
            }),
        },
        ToolDef {
            name: "create_content_type",
            description: "Create a new dynamic CMS content type. This writes a TOML schema file, \
                runs DB migration (CREATE TABLE), and hot-registers REST routes — all in one step. \
                **Read the `raisfast://content-type-schema-guide` resource first** to learn the \
                full schema grammar (field types, protocols, relations). Requires admin session.",
            input_schema: json!({
                "type": "object",
                "required": ["name", "singular", "plural", "table"],
                "properties": {
                    "name": { "type": "string", "description": "Display name (e.g. \"Product\")" },
                    "singular": { "type": "string", "description": "Singular identifier, lowercase + underscores" },
                    "plural": { "type": "string", "description": "Plural identifier" },
                    "table": { "type": "string", "description": "Database table name (globally unique)" },
                    "group": { "type": "string", "description": "Namespace group (empty = flat routes)" },
                    "description": { "type": "string", "description": "What this content type is for" },
                    "kind": { "type": "string", "enum": ["collection", "single"], "default": "collection" },
                    "slug_field": { "type": "string", "description": "Field to auto-generate slug from" },
                    "implements": {
                        "type": "array",
                        "description": "Protocols to enable. Options: ownable, timestampable, soft_deletable, versionable, lockable, sortable, expirable, nestable, statusable, metaable, tenantable. Use objects for configurable protocols: {\"name\":\"statusable\",\"values\":\"draft,published\",\"default\":\"draft\"}",
                        "items": {}
                    },
                    "fields": {
                        "type": "array",
                        "description": "Field definitions. Each field needs: name, field_type (text/richtext/integer/decimal/float/boolean/date/datetime/email/password/enum/uid/json/media/relation), and optional: required, unique, default, label, description, max_length, min, max, pattern, private, immutable, enum_values, target_field, relation_type (one_to_one/one_to_many/many_to_one/many_to_many/one_way/many_way), target, foreign_key, through, accept, max_count",
                        "items": { "type": "object" }
                    }
                }
            }),
        },
        ToolDef {
            name: "update_content_type",
            description: "Update an existing content type (e.g. add new fields). Adding fields \
                triggers ALTER TABLE — existing data is preserved. Only provided fields are updated. \
                Requires admin session.",
            input_schema: json!({
                "type": "object",
                "required": ["content_type"],
                "properties": {
                    "content_type": { "type": "string", "description": "Singular key of the content type to update" },
                    "description": { "type": "string" },
                    "slug_field": { "type": "string" },
                    "implements": { "type": "array", "items": {} },
                    "fields": { "type": "array", "description": "COMPLETE field list (replaces all fields)", "items": { "type": "object" } },
                    "indexes": { "type": "array", "description": "COMPLETE index list (replaces all indexes)", "items": { "type": "object" } }
                }
            }),
        },
        ToolDef {
            name: "delete_content_type",
            description: "Delete a content type by its singular key. Removes the TOML file and \
                unregisters routes. Does NOT drop the database table (data is preserved). \
                Requires admin session.",
            input_schema: json!({
                "type": "object",
                "required": ["content_type"],
                "properties": {
                    "content_type": { "type": "string", "description": "Singular key of the content type to delete" }
                }
            }),
        },
    ]
});

// ─── Tool implementations ──────────────────────────────────────────────

/// `list_content_types` — enumerate the dynamic CMS content types in the registry.
///
/// Blog posts are **excluded** here because they have dedicated tools
/// (`list_posts`, `get_post`, `create_post`) backed by the PostService.
async fn list_content_types(ctx: &McpContext) -> Result<Value, ErrorObject> {
    let out: Vec<Value> = ctx
        .state
        .content_type_registry
        .all()
        .iter()
        .map(|ct| {
            let fields: Vec<Value> = ct
                .fields
                .iter()
                .map(|f| {
                    json!({
                        "name": f.name,
                        "label": f.label,
                        "type": f.field_type,
                        "description": f.description,
                        "required": f.required,
                    })
                })
                .collect();
            json!({
                "name": ct.name,
                "description": ct.description,
                "singular": ct.singular,
                "plural": ct.plural,
                "group": ct.group,
                "table": ct.table,
                "kind": if ct.is_single() { "single" } else { "collection" },
                "fields": fields,
            })
        })
        .collect();

    Ok(super::truncate(json!(out), ctx.config.max_result_chars))
}

/// `list_entries` — paginated query of a dynamic content type.
async fn list_entries(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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
        .ok_or_else(|| ErrorObject::invalid_params(format!("unknown content type: {plural}")))?;

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

    Ok(super::truncate(
        json!({ "items": rows, "total": total, "content_type": plural }),
        ctx.config.max_result_chars,
    ))
}

/// `get_entry` — fetch one record by ID.
async fn get_entry(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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
        .ok_or_else(|| ErrorObject::invalid_params(format!("unknown content type: {plural}")))?;

    let id = crate::types::snowflake_id::parse_id(id_str)
        .map_err(|e| ErrorObject::invalid_params(format!("invalid id: {e}")))?;

    let row = ctx
        .repo
        .find_by_id(&ct, id, ctx.auth.tenant_id(), true)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?
        .ok_or_else(|| ErrorObject::new(-32001, format!("entry {id_str} not found")))?;

    Ok(super::truncate(row, ctx.config.max_result_chars))
}

/// `create_entry` — insert a record into a dynamic content type.
async fn create_entry(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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
        .ok_or_else(|| ErrorObject::invalid_params(format!("unknown content type: {plural}")))?;

    let save_ctx = crate::content_type::repository::SaveContext::from_auth(&ctx.auth);
    let created = ctx
        .repo
        .create(&ct, fields, ctx.auth.tenant_id(), &save_ctx)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    Ok(super::truncate(created, ctx.config.max_result_chars))
}

/// `list_posts` — blog post listing via the PostService.
async fn list_posts(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
    let page = args
        .get("page")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    let page_size = args
        .get("page_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(10)
        .clamp(1, 100);
    let q = args.get("q").and_then(|v| v.as_str());

    let (posts, total) = ctx
        .state
        .post_service
        .list(&ctx.auth, page, page_size, None, None, q)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    let summary: Vec<Value> = posts
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "title": p.title,
                "slug": p.slug,
                "status": format!("{:?}", p.status),
                "author": p.author_name,
                "excerpt": p.excerpt,
            })
        })
        .collect();

    Ok(super::truncate(
        json!({ "items": summary, "total": total, "page": page, "page_size": page_size }),
        ctx.config.max_result_chars,
    ))
}

/// `get_post` — fetch a single blog post by slug.
async fn get_post(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
    let slug = args
        .get("slug")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'slug'"))?;

    let post = ctx
        .state
        .post_service
        .get(&ctx.auth, slug)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    Ok(super::truncate(
        serde_json::to_value(&post).unwrap_or(json!({})),
        ctx.config.max_result_chars,
    ))
}

/// `create_post` — create a blog post via the PostService.
async fn create_post(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
    ctx.auth
        .ensure_author()
        .map_err(|e| ErrorObject::new(-32603, e.to_string()))?;

    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'title'"))?
        .to_string();
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'content'"))?
        .to_string();

    let req = crate::dto::CreatePostRequest {
        title,
        content,
        slug: args.get("slug").and_then(|v| v.as_str()).map(String::from),
        excerpt: args
            .get("excerpt")
            .and_then(|v| v.as_str())
            .map(String::from),
        status: args
            .get("status")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok()),
        cover_image: None,
        image_ids: None,
        category_id: None,
        tag_ids: None,
        meta_title: None,
        meta_description: None,
        og_title: None,
        og_description: None,
        og_image: None,
        canonical_url: None,
    };

    let post = ctx
        .state
        .post_service
        .create(&ctx.auth, req)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    Ok(json!({
        "id": post.id,
        "slug": post.slug,
        "title": post.title,
        "status": format!("{:?}", post.status),
        "created": true,
    }))
}

// ─── Content type management tools ─────────────────────────────────────

/// `create_content_type` — create a new dynamic CMS content type.
///
/// Replicates the admin REST handler flow: validate → save TOML → migrate DB →
/// register in the hot-reload registry. Requires admin privileges.
async fn create_content_type(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
    ctx.auth
        .ensure_admin()
        .map_err(|e| ErrorObject::new(-32603, e.to_string()))?;

    // Deserialize the arguments into CreateContentTypeRequest — serde does the
    // heavy lifting of mapping JSON → FieldSchema / ProtocolRef / etc.
    let req: crate::content_type::schema::CreateContentTypeRequest =
        serde_json::from_value(args.clone()).map_err(|e| {
            ErrorObject::invalid_params(format!(
                "invalid content type definition: {e}. \
                 Read the raisfast://content-type-schema-guide resource for the full schema."
            ))
        })?;

    // Build the full ContentTypeSchema (same logic as the admin handler)
    let schema = crate::content_type::schema::ContentTypeSchema {
        name: req.name,
        singular: req.singular.clone(),
        plural: req.plural,
        table: req.table.clone(),
        group: crate::content_type::schema::ContentTypeSchema::validate_group_name(&req.group)
            .map_err(|e| ErrorObject::invalid_params(e.to_string()))?,
        description: req.description,
        kind: req.kind,
        slug_field: req.slug_field,
        builtin: req.builtin,
        implements: req.implements,
        fields: req.fields,
        indexes: vec![],
        api: crate::content_type::schema::ApiConfig::default(),
        cached_column_names: None,
        cached_protocol_column_names: None,
        cached_behaviors: None,
        cached_declaration: None,
        cached_rules: None,
    };

    // Guard: protected table
    if crate::plugins::permissions::PermissionChecker::is_protected_table(
        &schema.table,
        &ctx.state.config.builtins.protected_tables(),
    ) {
        return Err(ErrorObject::invalid_params(format!(
            "table '{}' is a protected system table",
            schema.table
        )));
    }

    // Guard: already exists
    let registry_key = schema.registry_key();
    if ctx.state.content_type_registry.get(&registry_key).is_some() {
        return Err(ErrorObject::new(
            -32603,
            format!("content type '{registry_key}' already exists"),
        ));
    }

    // Save TOML → migrate DB → register
    let dir = std::path::Path::new(&ctx.state.config.content_type_dir);
    schema
        .save_to_dir(dir)
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    ctx.repo
        .migrate(&schema, &ctx.state.protocol_registry)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    let reserved = ctx.state.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = ctx.state.protocol_registry.names();
    ctx.state
        .content_type_registry
        .register(
            schema.clone(),
            &ctx.state.config.rule_engine,
            &reserved,
            &protocol_names,
            &ctx.state.protocol_registry,
        )
        .map_err(|e| ErrorObject::new(-32603, e.to_string()))?;

    tracing::info!(
        "MCP: created content type '{}' (table={}, {} fields)",
        schema.singular,
        schema.table,
        schema.fields.len()
    );

    Ok(json!({
        "created": true,
        "singular": schema.singular,
        "plural": schema.plural,
        "table": schema.table,
        "fields_count": schema.fields.len(),
        "registry_key": schema.registry_key(),
        "route_segment": schema.route_segment(),
        "message": format!(
            "Content type '{}' created. REST routes at /api/v1/cms/{} are now live.",
            schema.name, schema.route_segment()
        ),
    }))
}

/// `update_content_type` — update an existing content type (add fields, etc.)
async fn update_content_type(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
    ctx.auth
        .ensure_admin()
        .map_err(|e| ErrorObject::new(-32603, e.to_string()))?;

    let key = args
        .get("content_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'content_type' (singular key)"))?;

    let ct = ctx
        .state
        .content_type_registry
        .get(key)
        .ok_or_else(|| ErrorObject::new(-32001, format!("content type '{key}' not found")))?
        .clone(); // Arc<ContentTypeSchema> → need owned for mutation

    // Deserialize only the update fields
    let req: crate::content_type::schema::UpdateContentTypeRequest =
        serde_json::from_value(args.clone())
            .map_err(|e| ErrorObject::invalid_params(format!("invalid update definition: {e}")))?;

    let mut updated = ct.as_ref().clone();

    if let Some(name) = req.name {
        updated.name = name;
    }
    if let Some(description) = req.description {
        updated.description = description;
    }
    if let Some(slug_field) = req.slug_field {
        updated.slug_field = slug_field;
    }
    if let Some(implements) = req.implements {
        updated.implements = implements;
    }
    if let Some(fields) = req.fields {
        updated.fields = fields;
    }
    if let Some(indexes) = req.indexes {
        updated.indexes = indexes;
    }

    // Save → migrate → re-register
    let dir = std::path::Path::new(&ctx.state.config.content_type_dir);
    updated
        .save_to_dir(dir)
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    ctx.repo
        .migrate(&updated, &ctx.state.protocol_registry)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    let reserved = ctx.state.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = ctx.state.protocol_registry.names();
    ctx.state
        .content_type_registry
        .register(
            updated.clone(),
            &ctx.state.config.rule_engine,
            &reserved,
            &protocol_names,
            &ctx.state.protocol_registry,
        )
        .map_err(|e| ErrorObject::new(-32603, e.to_string()))?;

    Ok(json!({
        "updated": true,
        "singular": updated.singular,
        "fields_count": updated.fields.len(),
    }))
}

/// `delete_content_type` — remove a content type (TOML + registry, NOT DB table)
async fn delete_content_type(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
    ctx.auth
        .ensure_admin()
        .map_err(|e| ErrorObject::new(-32603, e.to_string()))?;

    let key = args
        .get("content_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'content_type' (singular key)"))?;

    let ct = ctx
        .state
        .content_type_registry
        .get(key)
        .ok_or_else(|| ErrorObject::new(-32001, format!("content type '{key}' not found")))?
        .clone(); // Arc<ContentTypeSchema>

    // Delete TOML file
    let path = std::path::Path::new(&ctx.state.config.content_type_dir).join(ct.toml_filename());
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| ErrorObject::internal_error(format!("cannot delete {:?}: {e}", path)))?;
    }

    // Unregister from memory
    ctx.state.content_type_registry.unregister(key);

    Ok(json!({
        "deleted": true,
        "singular": ct.singular,
        "table_preserved": ct.table,
        "message": format!(
            "Content type '{}' deleted. Database table '{}' is preserved (not dropped).",
            ct.singular, ct.table
        ),
    }))
}

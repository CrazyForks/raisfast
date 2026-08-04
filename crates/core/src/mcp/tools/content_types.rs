//! Content type management tools: list, create, update, delete content types.

use serde_json::{Value, json};

use crate::mcp::jsonrpc::ErrorObject;
use crate::mcp::{McpContext, truncate};

/// Register all content-type tools.
pub fn register(reg: &mut crate::mcp::registry::ToolRegistry) {
    reg.register(ListContentTypes);
    reg.register(CreateContentType);
    reg.register(UpdateContentType);
    reg.register(DeleteContentType);
}

// ═════════════════════════════════════════════════════════════════════════
// list_content_types
// ═════════════════════════════════════════════════════════════════════════

pub struct ListContentTypes;

crate::impl_tool!(
    ListContentTypes,
    "list_content_types",
    "List all dynamic CMS content types (defined by TOML schema files). Returns each type's \
    name, description, fields, and keys. Blog posts are NOT included here — use the dedicated \
    `list_posts`/`create_post` tools for those. Call this first to discover what data the \
    raisfast instance manages.",
    { "type": "object", "properties": {} }
);

impl ListContentTypes {
    async fn run(ctx: &McpContext, _args: &Value) -> Result<Value, ErrorObject> {
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

        Ok(truncate(json!(out), ctx.config.max_result_chars))
    }
}

// ═════════════════════════════════════════════════════════════════════════
// create_content_type
// ═════════════════════════════════════════════════════════════════════════

pub struct CreateContentType;

crate::impl_tool!(
    CreateContentType,
    "create_content_type",
    "Create a new dynamic CMS content type. This writes a TOML schema file, runs DB migration \
    (CREATE TABLE), and hot-registers REST routes — all in one step. **Read the \
    `raisfast://content-type-schema-guide` resource first** to learn the full schema grammar \
    (field types, protocols, relations). Requires admin session.",
    {
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
                "description": "Field definitions. Each field needs: name, field_type (text/richtext/integer/decimal/float/boolean/date/datetime/email/password/enum/uid/json/media/relation), and optional: required, unique, default, label, description, max_length, min, max, pattern, private, immutable, enum_values, target_field, relation_type, target, foreign_key, through, accept, max_count",
                "items": { "type": "object" }
            }
        }
    }
);

impl CreateContentType {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
        ctx.auth
            .ensure_admin()
            .map_err(|e| ErrorObject::new(-32603, e.to_string()))?;

        let req: crate::content_type::schema::CreateContentTypeRequest =
            serde_json::from_value(args.clone()).map_err(|e| {
                ErrorObject::invalid_params(format!(
                    "invalid content type definition: {e}. \
                     Read the raisfast://content-type-schema-guide resource for the full schema."
                ))
            })?;

        let schema = crate::content_type::schema::ContentTypeSchema {
            name: req.name,
            singular: req.singular.clone(),
            plural: req.plural,
            table: req.table.clone(),
            group: crate::content_type::schema::ContentTypeSchema::validate_group_name(&req.group)
                .map_err(|e| ErrorObject::invalid_params(e.to_string()))?,
            description: req.description,
            icon: req.icon,
            color: req.color,
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

        if crate::plugins::permissions::PermissionChecker::is_protected_table(
            &schema.table,
            &ctx.state.config.builtins.protected_tables(),
        ) {
            return Err(ErrorObject::invalid_params(format!(
                "table '{}' is a protected system table",
                schema.table
            )));
        }

        let registry_key = schema.registry_key();
        if ctx.state.content_type_registry.get(&registry_key).is_some() {
            return Err(ErrorObject::new(
                -32603,
                format!("content type '{registry_key}' already exists"),
            ));
        }

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
}

// ═════════════════════════════════════════════════════════════════════════
// update_content_type
// ═════════════════════════════════════════════════════════════════════════

pub struct UpdateContentType;

crate::impl_tool!(
    UpdateContentType,
    "update_content_type",
    "Update an existing content type (e.g. add new fields). Adding fields triggers ALTER TABLE \
    — existing data is preserved. Only provided fields are updated. Requires admin session.",
    {
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
    }
);

impl UpdateContentType {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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
            .clone();

        let req: crate::content_type::schema::UpdateContentTypeRequest =
            serde_json::from_value(args.clone()).map_err(|e| {
                ErrorObject::invalid_params(format!("invalid update definition: {e}"))
            })?;

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
}

// ═════════════════════════════════════════════════════════════════════════
// delete_content_type
// ═════════════════════════════════════════════════════════════════════════

pub struct DeleteContentType;

crate::impl_tool!(
    DeleteContentType,
    "delete_content_type",
    "Delete a content type by its singular key. Removes the TOML file and unregisters routes. \
    Does NOT drop the database table (data is preserved). Requires admin session.",
    {
        "type": "object",
        "required": ["content_type"],
        "properties": {
            "content_type": { "type": "string", "description": "Singular key of the content type to delete" }
        }
    }
);

impl DeleteContentType {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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
            .clone();

        let path =
            std::path::Path::new(&ctx.state.config.content_type_dir).join(ct.toml_filename());
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                ErrorObject::internal_error(format!("cannot delete {:?}: {e}", path))
            })?;
        }

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
}

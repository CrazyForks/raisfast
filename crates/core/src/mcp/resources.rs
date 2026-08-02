//! MCP Resources — URI-addressable, read-only data that AI clients can browse.
//!
//! Resources are the right primitive for data the AI inspects on demand (no
//! side effects, located by URI). Tools are reserved for parameterized
//! operations. Exposing schemas as resources lets an assistant discover the
//! instance's data model without consuming a tool-call.

use serde_json::{Value, json};

use super::McpContext;
use super::jsonrpc::ErrorObject;

/// `resources/list` — enumerate the static resources this server exposes.
pub(crate) fn list_resources(_ctx: &McpContext) -> Value {
    let resources = vec![
        json!({
            "uri": "raisfast://content-types",
            "name": "Content Type Schemas",
            "description": "Full definitions of every dynamic CMS content type. \
                Read this to learn what fields each type has.",
            "mimeType": "application/json",
        }),
        json!({
            "uri": "raisfast://content-type-schema-guide",
            "name": "Content Type Schema Guide",
            "description": "Complete reference for defining content types: all 17 field types, \
                11 protocols, 6 relation types, API access rules, and TOML syntax examples. \
                **Read this BEFORE calling `create_content_type`** to learn the full schema.",
            "mimeType": "text/markdown",
        }),
        json!({
            "uri": "raisfast://routes",
            "name": "Registered HTTP Routes",
            "description": "Every route registered on this raisfast instance (REST + dynamic CMS).",
            "mimeType": "application/json",
        }),
    ];
    json!({ "resources": resources })
}

/// `resources/read` — fetch the content of a resource by URI.
pub(crate) async fn read_resource(ctx: &McpContext, uri: &str) -> Result<Value, ErrorObject> {
    let body = match uri {
        "raisfast://content-types" => read_content_types(ctx).await?,
        "raisfast://content-type-schema-guide" => Value::String(content_type_schema_guide()),
        "raisfast://routes" => read_routes(ctx),
        other => {
            // Resource templates: raisfast://content-types/{key}
            if let Some(key) = other.strip_prefix("raisfast://content-types/") {
                read_one_content_type(ctx, key)?
            } else {
                return Err(ErrorObject::new(
                    -32002,
                    format!("unknown resource: {other}"),
                ));
            }
        }
    };

    let text = if body.is_string() {
        body.as_str().unwrap_or_default().to_string()
    } else {
        serde_json::to_string_pretty(&body).unwrap_or_default()
    };

    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": text,
        }]
    }))
}

async fn read_content_types(ctx: &McpContext) -> Result<Value, ErrorObject> {
    let schemas: Vec<Value> = ctx
        .state
        .content_type_registry
        .all()
        .iter()
        .map(|ct| {
            json!({
                "name": ct.name,
                "singular": ct.singular,
                "plural": ct.plural,
                "group": ct.group,
                "table": ct.table,
                "kind": if ct.is_single() { "single" } else { "collection" },
                "fields": ct.fields,
                "implements": ct.implements,
            })
        })
        .collect();
    Ok(super::truncate(json!(schemas), ctx.config.max_result_chars))
}

fn read_one_content_type(ctx: &McpContext, key: &str) -> Result<Value, ErrorObject> {
    let ct = ctx
        .state
        .content_type_registry
        .get(key)
        .or_else(|| ctx.state.content_type_registry.get_by_plural(key))
        .or_else(|| ctx.state.content_type_registry.get_by_table(key))
        .ok_or_else(|| ErrorObject::new(-32002, format!("unknown content type: {key}")))?;
    Ok(json!({
        "name": ct.name,
        "singular": ct.singular,
        "plural": ct.plural,
        "group": ct.group,
        "table": ct.table,
        "kind": if ct.is_single() { "single" } else { "collection" },
        "fields": ct.fields,
        "implements": ct.implements,
        "indexes": ct.indexes,
    }))
}

fn read_routes(ctx: &McpContext) -> Value {
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
    super::truncate(json!(routes), ctx.config.max_result_chars)
}

/// The full content type schema reference, distilled from the dev guide.
///
/// This is returned as a resource so AI assistants can read it before calling
/// `create_content_type`, giving them the complete grammar of TOML definitions:
/// field types, protocols, relations, API rules, and validation constraints.
fn content_type_schema_guide() -> String {
    r#"# Content Type Schema Guide

## Quick Start

A content type is defined by its name, identifiers, fields, and protocols. Here's a minimal example:

```json
{
  "name": "Product",
  "singular": "product",
  "plural": "products",
  "table": "products",
  "description": "商品 / Products catalog",
  "implements": ["ownable", "timestampable", "tenantable"],
  "fields": [
    { "name": "name", "field_type": "text", "required": true, "max_length": 200 },
    { "name": "price", "field_type": "decimal", "required": true, "min": 0 }
  ]
}
```

## Top-Level Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | ✅ | Display name (e.g. "Product") |
| `singular` | string | ✅ | Single identifier, lowercase + underscores only (e.g. "product") |
| `plural` | string | ✅ | Plural identifier (e.g. "products") |
| `table` | string | ✅ | Database table name, globally unique (e.g. "products") |
| `group` | string | ❌ | Namespace group for grouped routes (empty = flat) |
| `description` | string | ❌ | Human-readable description of this content type |
| `kind` | string | ❌ | "collection" (default, multiple records) or "single" (one record only) |
| `slug_field` | string | ❌ | Which field to auto-generate slug from |
| `builtin` | bool | ❌ | Whether this is a built-in type (default false) |
| `implements` | array | ❌ | Protocol list (see below) |
| `fields` | array | ❌ | Field definitions (see below) |

## Field Types (17 types)

| Type | Description | SQL |
|------|-------------|-----|
| `text` | Plain text | TEXT |
| `richtext` | Rich text / HTML | TEXT |
| `integer` | Integer | INTEGER |
| `bigint` | Big integer | INTEGER |
| `decimal` | Precise decimal | REAL |
| `float` | Floating point | REAL |
| `boolean` | Boolean (0/1) | BOOLEAN |
| `date` | ISO 8601 date | TEXT |
| `datetime` | ISO 8601 datetime | TEXT |
| `time` | ISO 8601 time | TEXT |
| `email` | Email address | TEXT |
| `password` | Password (hashed) | TEXT |
| `enum` | Enum value | TEXT |
| `uid` | Auto-generated slug/UUID from `target_field` | TEXT |
| `json` | Arbitrary JSON | TEXT |
| `media` | File attachment (URL) | TEXT |
| `relation` | Relation to another content type | TEXT (FK ID) |

## Field Properties

| Property | Applies to | Description |
|----------|-----------|-------------|
| `name` | all | Field identifier (lowercase + underscores) |
| `field_type` | all | One of the 17 types above |
| `required` | all | Must be non-empty |
| `unique` | all | Unique constraint |
| `default` | all | Default value |
| `private` | all | Hidden in public API, visible in admin API |
| `immutable` | all | Cannot be changed after creation |
| `label` | all | Admin UI display label |
| `description` | all | Human-readable field description |
| `max_length` | text/email/password | Maximum string length |
| `min` | integer/decimal/float | Minimum numeric value |
| `max` | integer/decimal/float | Maximum numeric value |
| `pattern` | text/email | Regex validation |
| `enum_values` | enum | List of allowed values |
| `target_field` | uid | Source field for slug generation |
| `relation_type` | relation | See relation types below |
| `target` | relation | Target content type's plural name |
| `foreign_key` | relation | FK column name (default: `{field}_id`) |
| `through` | relation | Junction table name (many_to_many only) |
| `accept` | media | Allowed MIME types (e.g. `["image/*"]`) |
| `max_count` | media | Maximum file count (default 1) |

## Relation Types (6 types)

| Type | Description |
|------|-------------|
| `one_to_one` | Source table FK, 1:1 |
| `one_to_many` | Target table FK (reverse lookup) |
| `many_to_one` | Source table FK (the owning side) |
| `many_to_many` | Junction table |
| `one_way` | Source table FK, no back-link |
| `many_way` | Source table FK, no back-link |

### Relation example
```json
{
  "name": "category",
  "field_type": "relation",
  "relation_type": "many_to_one",
  "target": "categories",
  "foreign_key": "category_id",
  "required": true
}
```

### many_to_many example
```json
{
  "name": "tags",
  "field_type": "relation",
  "relation_type": "many_to_many",
  "target": "tags",
  "through": "article_tags"
}
```

## Protocols (11 built-in, use in `implements`)

| Protocol | Injected columns | Description |
|----------|-----------------|-------------|
| `ownable` | `created_by`, `updated_by` | Track who created/updated records |
| `timestampable` | `created_at`, `updated_at` | Auto timestamps |
| `soft_deletable` | `deleted_at`, `deleted_by` | Soft delete (UPDATE instead of DELETE) |
| `versionable` | `version` | Revision history with /revisions API |
| `lockable` | `lock_version` | Optimistic locking (409 on conflict) |
| `sortable` | — (config-driven) | Default sort order |
| `expirable` | `expires_at` | TTL expiry, auto-filtered in queries |
| `nestable` | `parent_id`, `depth`, `position` | Tree/hierarchy |
| `statusable` | `status` | Configurable status field |
| `metaable` | `__meta` | Dynamic JSON metadata |
| `tenantable` | `tenant_id` | Multi-tenant isolation |

### Protocol with config
Some protocols accept configuration:

```json
"implements": [
  "ownable",
  "timestampable",
  { "name": "sortable", "field": "priority", "direction": "desc" },
  { "name": "statusable", "values": "draft,published,archived", "default": "draft" }
]
```

- **sortable**: `field` (default "created_at"), `direction` ("asc"/"desc", default "desc")
- **statusable**: `values` (comma-separated), `default`, `mode` ("string"/"numeric")

## Validation Rules

- `singular`, `plural`, `table`, `group`: only `a-zA-Z0-9_`, must be globally unique
- Field names: only `a-zA-Z0-9_`
- `table` cannot collide with protected system tables
- If any API endpoint uses `access: "owner"`, the type MUST implement `ownable` (for `created_by`)
- `singular`/`plural` cannot collide with reserved route segments (auth, posts, users, etc.)

## Complete Example

```json
{
  "name": "Article",
  "singular": "article",
  "plural": "articles",
  "table": "articles",
  "description": "博客文章",
  "slug_field": "title",
  "implements": [
    "ownable",
    "timestampable",
    "soft_deletable",
    "versionable",
    "lockable",
    { "name": "sortable", "field": "created_at", "direction": "desc" },
    { "name": "statusable", "values": "draft,published,archived", "default": "draft" },
    "tenantable"
  ],
  "fields": [
    { "name": "title", "field_type": "text", "required": true, "max_length": 200, "label": "标题" },
    { "name": "slug", "field_type": "uid", "target_field": "title", "unique": true },
    { "name": "content", "field_type": "richtext", "required": true },
    { "name": "excerpt", "field_type": "text", "max_length": 500 },
    { "name": "featured_image", "field_type": "media", "accept": ["image/*"] },
    {
      "name": "category", "field_type": "relation",
      "relation_type": "many_to_one", "target": "categories",
      "foreign_key": "category_id"
    },
    {
      "name": "tags", "field_type": "relation",
      "relation_type": "many_to_many", "target": "tags",
      "through": "article_tags"
    }
  ]
}
```
"#
    .to_string()
}

//! Content type resources: schema listings, individual schemas, and the full
//! schema guide that teaches AI clients the TOML definition grammar.

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::mcp::McpContext;
use crate::mcp::jsonrpc::ErrorObject;
use crate::mcp::registry::{ResourceMeta, ResourceProvider};
use crate::mcp::truncate;

/// Provider for all content-type-related resources.
pub struct ContentTypeProvider;

impl ResourceProvider for ContentTypeProvider {
    fn list(&self, _ctx: &McpContext) -> Vec<ResourceMeta> {
        vec![
            ResourceMeta {
                uri: "raisfast://content-types".to_string(),
                name: "Content Type Schemas".to_string(),
                description: "Full definitions of every dynamic CMS content type. \
                    Read this to learn what fields each type has."
                    .to_string(),
                mime_type: "application/json".to_string(),
            },
            ResourceMeta {
                uri: "raisfast://content-type-schema-guide".to_string(),
                name: "Content Type Schema Guide".to_string(),
                description: "Complete reference for defining content types: all 17 field types, \
                    11 protocols, 6 relation types, API access rules, and TOML syntax examples. \
                    **Read this BEFORE calling `create_content_type`** to learn the full schema."
                    .to_string(),
                mime_type: "text/markdown".to_string(),
            },
        ]
    }

    fn read<'a>(
        &'a self,
        ctx: &'a McpContext,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Option<Value>, ErrorObject>> {
        Box::pin(async move {
            match uri {
                "raisfast://content-types" => Ok(Some(read_all_schemas(ctx).await?)),
                "raisfast://content-type-schema-guide" => Ok(Some(Value::String(schema_guide()))),
                other => {
                    // Template: raisfast://content-types/{key}
                    if let Some(key) = other.strip_prefix("raisfast://content-types/") {
                        Ok(Some(read_one_schema(ctx, key)?))
                    } else {
                        Ok(None)
                    }
                }
            }
        })
    }
}

async fn read_all_schemas(ctx: &McpContext) -> Result<Value, ErrorObject> {
    let schemas: Vec<Value> = ctx
        .state
        .content_type_registry
        .all()
        .iter()
        .map(|ct| {
            json!({
                "name": ct.name,
                "description": ct.description,
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
    Ok(truncate(json!(schemas), ctx.config.max_result_chars))
}

fn read_one_schema(ctx: &McpContext, key: &str) -> Result<Value, ErrorObject> {
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

/// The full content type schema reference, distilled from the dev guide.
fn schema_guide() -> String {
    r#"# Content Type Schema Guide

## Quick Start

A content type is defined by its name, identifiers, fields, and protocols:

```json
{
  "name": "Product",
  "singular": "product",
  "plural": "products",
  "table": "products",
  "description": "Products catalog",
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
| `name` | string | yes | Display name |
| `singular` | string | yes | Lowercase identifier (a-z, 0-9, _) |
| `plural` | string | yes | Plural identifier |
| `table` | string | yes | DB table name (globally unique) |
| `group` | string | no | Namespace for grouped routes |
| `description` | string | no | Human description |
| `kind` | string | no | "collection" (default) or "single" |
| `slug_field` | string | no | Field to auto-generate slug from |
| `implements` | array | no | Protocol list (see below) |
| `fields` | array | no | Field definitions |

## Field Types (17)

text, richtext, integer, bigint, decimal, float, boolean, date, datetime, time,
email, password, enum, uid, json, media, relation

## Field Properties

| Property | Applies to | Description |
|----------|-----------|-------------|
| `name` | all | Field identifier |
| `field_type` | all | One of the 17 types |
| `required` | all | Must be non-empty |
| `unique` | all | Unique constraint |
| `default` | all | Default value |
| `private` | all | Hidden in public API |
| `immutable` | all | Cannot change after creation |
| `label` | all | Admin UI label |
| `description` | all | Field description |
| `max_length` | text/email/password | Max string length |
| `min` / `max` | numeric | Range bounds |
| `pattern` | text/email | Regex validation |
| `enum_values` | enum | Allowed values |
| `target_field` | uid | Source field for slug |
| `relation_type` | relation | one_to_one / one_to_many / many_to_one / many_to_many / one_way / many_way |
| `target` | relation | Target content type's plural |
| `foreign_key` | relation | FK column (default: {field}_id) |
| `through` | relation | Junction table (many_to_many) |
| `accept` | media | Allowed MIME types |
| `max_count` | media | Max files (default 1) |

## Protocols (11)

| Protocol | Columns | Description |
|----------|---------|-------------|
| `ownable` | created_by, updated_by | Track owners |
| `timestampable` | created_at, updated_at | Auto timestamps |
| `soft_deletable` | deleted_at, deleted_by | Soft delete |
| `versionable` | version | Revision history |
| `lockable` | lock_version | Optimistic locking |
| `sortable` | — | Default sort (config: field, direction) |
| `expirable` | expires_at | TTL expiry |
| `nestable` | parent_id, depth, position | Tree structure |
| `statusable` | status | Configurable status |
| `metaable` | __meta | JSON metadata |
| `tenantable` | tenant_id | Multi-tenant |

### Configurable protocols

```json
"implements": [
  {"name": "sortable", "field": "priority", "direction": "desc"},
  {"name": "statusable", "values": "draft,published,archived", "default": "draft"}
]
```

## Validation Rules

- Identifiers: only a-z, 0-9, underscore
- Table name: globally unique, not a protected system table
- If using access "owner", must implement "ownable"

## Complete Example

```json
{
  "name": "Article",
  "singular": "article",
  "plural": "articles",
  "table": "articles",
  "description": "Blog articles",
  "slug_field": "title",
  "implements": [
    "ownable", "timestampable", "soft_deletable", "versionable", "lockable", "tenantable",
    {"name": "sortable", "field": "created_at", "direction": "desc"},
    {"name": "statusable", "values": "draft,published,archived", "default": "draft"}
  ],
  "fields": [
    {"name": "title", "field_type": "text", "required": true, "max_length": 200},
    {"name": "slug", "field_type": "uid", "target_field": "title", "unique": true},
    {"name": "content", "field_type": "richtext", "required": true},
    {"name": "category", "field_type": "relation", "relation_type": "many_to_one", "target": "categories", "foreign_key": "category_id"},
    {"name": "tags", "field_type": "relation", "relation_type": "many_to_many", "target": "tags", "through": "article_tags"}
  ]
}
```
"#.to_string()
}

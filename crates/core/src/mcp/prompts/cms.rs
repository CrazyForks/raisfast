//! CMS-related prompts: content_type_wizard, audit_content.

use futures::future::BoxFuture;
use serde_json::Value;

use crate::mcp::McpContext;
use crate::mcp::jsonrpc::ErrorObject;
use crate::mcp::registry::{
    PromptArgMeta, PromptContent, PromptMessage, PromptMeta, PromptProvider, PromptRole,
};

/// Provider for CMS content-type prompts.
pub struct CmsPrompts;

impl PromptProvider for CmsPrompts {
    fn list(&self) -> Vec<PromptMeta> {
        vec![
            PromptMeta {
                name: "content_type_wizard".to_string(),
                title: Some("Content Type Wizard".to_string()),
                description: "Guide the user through designing a new content type. Reads the \
                    schema guide resource and existing types, then helps design fields, protocols, \
                    and relations for the new type."
                    .to_string(),
                arguments: vec![PromptArgMeta {
                    name: "description".to_string(),
                    description: "What you want the content type to do (e.g. 'a product catalog \
                        with categories and reviews')"
                        .to_string(),
                    required: true,
                }],
            },
            PromptMeta {
                name: "audit_content".to_string(),
                title: Some("Content Audit".to_string()),
                description: "Audit the health of all content types: field completeness, missing \
                    protocols, unused types, and data quality. Returns a structured report."
                    .to_string(),
                arguments: vec![],
            },
        ]
    }

    fn get<'a>(
        &'a self,
        ctx: &'a McpContext,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Option<Vec<PromptMessage>>, ErrorObject>> {
        Box::pin(async move {
            match name {
                "content_type_wizard" => Ok(Some(content_type_wizard(ctx, args).await?)),
                "audit_content" => Ok(Some(audit_content(ctx).await?)),
                _ => Ok(None),
            }
        })
    }
}

async fn content_type_wizard(
    ctx: &McpContext,
    args: &Value,
) -> Result<Vec<PromptMessage>, ErrorObject> {
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'description' argument"))?;

    // Gather existing types so the wizard doesn't suggest collisions.
    let existing: Vec<String> = ctx
        .state
        .content_type_registry
        .all()
        .iter()
        .map(|ct| format!("{} ({})", ct.name, ct.plural))
        .collect();
    let existing_str = if existing.is_empty() {
        "(none)".to_string()
    } else {
        existing.join(", ")
    };

    let instruction = format!(
        "You are a raisfast content type architect. Design a new content type based on \
        the user's description.\n\n\
        **User's request:** {description}\n\n\
        **Existing content types (do not collide):** {existing_str}\n\n\
        Before designing, read the schema guide:\n\
        - Use `resources/read` with URI `raisfast://content-type-schema-guide` to learn the \
          full grammar (17 field types, 11 protocols, 6 relation types).\n\n\
        Then design the content type and produce a JSON definition ready for the \
        `create_content_type` tool. Include:\n\
        1. A name, singular, plural, table, and description.\n\
        2. Appropriate protocols (e.g. timestampable, ownable, soft_deletable).\n\
        3. All fields with correct types, constraints, and labels.\n\
        4. Any relations to existing content types (use their plural keys as targets).\n\
        5. A recommended slug_field.\n\n\
        Present the JSON and briefly explain your choices. Then ask the user if they want \
        you to call `create_content_type` to create it."
    );

    Ok(vec![
        PromptMessage {
            role: PromptRole::User,
            content: PromptContent::ResourceLink {
                uri: "raisfast://content-type-schema-guide".to_string(),
                name: "Content Type Schema Guide".to_string(),
            },
        },
        PromptMessage {
            role: PromptRole::User,
            content: PromptContent::Text(instruction),
        },
    ])
}

async fn audit_content(ctx: &McpContext) -> Result<Vec<PromptMessage>, ErrorObject> {
    let types = ctx.state.content_type_registry.all();

    if types.is_empty() {
        return Ok(vec![PromptMessage {
            role: PromptRole::User,
            content: PromptContent::Text(
                "There are no dynamic content types registered. Use the content_type_wizard \
                prompt or the create_content_type tool to create one."
                    .to_string(),
            ),
        }]);
    }

    let type_summaries: Vec<String> = types
        .iter()
        .map(|ct| {
            let protocols: Vec<&str> = ct.implements.iter().map(|p| p.name()).collect();
            let field_names: Vec<&str> = ct.fields.iter().map(|f| f.name.as_str()).collect();
            format!(
                "- **{}** (`{}` / table: `{}`)\n  Kind: {}\n  Protocols: {}\n  Fields ({}): {}",
                ct.name,
                ct.plural,
                ct.table,
                if ct.is_single() {
                    "single"
                } else {
                    "collection"
                },
                if protocols.is_empty() {
                    "(none)".to_string()
                } else {
                    protocols.join(", ")
                },
                ct.fields.len(),
                field_names.join(", "),
            )
        })
        .collect();

    let types_str = type_summaries.join("\n");

    let instruction = format!(
        "You are a CMS data architect auditing the health of a raisfast instance's content types.\n\n\
        **Registered content types:**\n{types_str}\n\n\
        Perform a health audit:\n\
        1. **Schema quality** — Are field types appropriate? Any missing constraints (required, max_length)?\n\
        2. **Protocol coverage** — Which types are missing recommended protocols (timestampable, ownable)?\n\
        3. **Naming consistency** — Are singular/plural/table names consistent and well-formed?\n\
        4. **Relations** — Are foreign keys named consistently? Any orphaned relations?\n\
        5. **Recommendations** — Suggest 3 concrete improvements with priority (high/medium/low).\n\n\
        Use `list_entries` to spot-check data quality if needed. Present the report in Markdown."
    );

    Ok(vec![PromptMessage {
        role: PromptRole::User,
        content: PromptContent::Text(instruction),
    }])
}

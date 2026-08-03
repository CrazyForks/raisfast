//! Blog post tools: list, get, create posts via PostService.

use serde_json::{Value, json};

use crate::mcp::jsonrpc::ErrorObject;
use crate::mcp::{McpContext, truncate};

/// Register all blog post tools.
pub fn register(reg: &mut crate::mcp::registry::ToolRegistry) {
    reg.register(ListPosts);
    reg.register(GetPost);
    reg.register(CreatePost);
}

// ═════════════════════════════════════════════════════════════════════════
// list_posts
// ═════════════════════════════════════════════════════════════════════════

pub struct ListPosts;

crate::impl_tool!(
    ListPosts,
    "list_posts",
    "List blog posts (published). Supports keyword search and pagination.",
    {
        "type": "object",
        "properties": {
            "page": { "type": "integer", "default": 1 },
            "page_size": { "type": "integer", "default": 10 },
            "q": { "type": "string", "description": "Optional keyword search" }
        }
    }
);

impl ListPosts {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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

        Ok(truncate(
            json!({ "items": summary, "total": total, "page": page, "page_size": page_size }),
            ctx.config.max_result_chars,
        ))
    }
}

// ═════════════════════════════════════════════════════════════════════════
// get_post
// ═════════════════════════════════════════════════════════════════════════

pub struct GetPost;

crate::impl_tool!(
    GetPost,
    "get_post",
    "Fetch a single blog post by its slug.",
    {
        "type": "object",
        "required": ["slug"],
        "properties": { "slug": { "type": "string" } }
    }
);

impl GetPost {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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

        Ok(truncate(
            serde_json::to_value(&post).unwrap_or(json!({})),
            ctx.config.max_result_chars,
        ))
    }
}

// ═════════════════════════════════════════════════════════════════════════
// create_post
// ═════════════════════════════════════════════════════════════════════════

pub struct CreatePost;

crate::impl_tool!(
    CreatePost,
    "create_post",
    "Create a new blog post. Requires an author/admin MCP session.",
    {
        "type": "object",
        "required": ["title", "content"],
        "properties": {
            "title": { "type": "string" },
            "content": { "type": "string", "description": "Markdown body" },
            "slug": { "type": "string" },
            "excerpt": { "type": "string" },
            "status": { "type": "string", "enum": ["draft", "published", "scheduled"], "default": "draft" }
        }
    }
);

impl CreatePost {
    async fn run(ctx: &McpContext, args: &Value) -> Result<Value, ErrorObject> {
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
}

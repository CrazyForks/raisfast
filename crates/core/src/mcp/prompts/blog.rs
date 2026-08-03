//! Blog-related prompts: draft_post, seo_review, summarize_latest.

use futures::future::BoxFuture;
use serde_json::Value;

use crate::mcp::McpContext;
use crate::mcp::jsonrpc::ErrorObject;
use crate::mcp::registry::{
    PromptArgMeta, PromptContent, PromptMessage, PromptMeta, PromptProvider, PromptRole,
};

/// Provider for blog post prompts.
pub struct BlogPrompts;

impl PromptProvider for BlogPrompts {
    fn list(&self) -> Vec<PromptMeta> {
        vec![
            PromptMeta {
                name: "draft_post".to_string(),
                title: Some("Draft Blog Post".to_string()),
                description: "Draft a new blog post. The assistant will suggest a title, \
                    generate a draft body, and fill in SEO metadata based on the provided topic."
                    .to_string(),
                arguments: vec![
                    PromptArgMeta {
                        name: "topic".to_string(),
                        description: "The topic or subject of the blog post".to_string(),
                        required: true,
                    },
                    PromptArgMeta {
                        name: "tone".to_string(),
                        description: "Writing tone: 'formal', 'casual', 'technical' (default: casual)"
                            .to_string(),
                        required: false,
                    },
                    PromptArgMeta {
                        name: "length".to_string(),
                        description: "Target length: 'short' (~500 words), 'medium' (~1000), 'long' (~2000)"
                            .to_string(),
                        required: false,
                    },
                ],
            },
            PromptMeta {
                name: "seo_review".to_string(),
                title: Some("SEO Review".to_string()),
                description: "Review an existing blog post for SEO and readability. Fetches the \
                    post by slug and analyzes its title, meta, content structure, and keyword usage."
                    .to_string(),
                arguments: vec![PromptArgMeta {
                    name: "slug".to_string(),
                    description: "The slug of the post to review".to_string(),
                    required: true,
                }],
            },
            PromptMeta {
                name: "summarize_latest".to_string(),
                title: Some("Summarize Latest Posts".to_string()),
                description: "Fetch the latest blog posts and produce a digest summary. Useful \
                    for newsletters or weekly roundups.".to_string(),
                arguments: vec![PromptArgMeta {
                    name: "count".to_string(),
                    description: "Number of recent posts to include (default: 5, max: 20)"
                        .to_string(),
                    required: false,
                }],
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
                "draft_post" => Ok(Some(draft_post(ctx, args).await?)),
                "seo_review" => Ok(Some(seo_review(ctx, args).await?)),
                "summarize_latest" => Ok(Some(summarize_latest(ctx, args).await?)),
                _ => Ok(None),
            }
        })
    }
}

async fn draft_post(_ctx: &McpContext, args: &Value) -> Result<Vec<PromptMessage>, ErrorObject> {
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'topic' argument"))?;
    let tone = args
        .get("tone")
        .and_then(|v| v.as_str())
        .unwrap_or("casual");
    let length = args
        .get("length")
        .and_then(|v| v.as_str())
        .unwrap_or("medium");

    let target_words = match length {
        "short" => 500,
        "medium" => 1000,
        "long" => 2000,
        _ => 1000,
    };

    let instruction = format!(
        "You are a content writer for a blog powered by raisfast. \
        Write a blog post draft on the following topic.\n\n\
        **Topic:** {topic}\n\
        **Tone:** {tone}\n\
        **Target length:** ~{target_words} words\n\n\
        Requirements:\n\
        1. Generate a compelling, SEO-friendly title.\n\
        2. Write the full body in Markdown.\n\
        3. Suggest a URL slug (lowercase, hyphen-separated).\n\
        4. Write a 1–2 sentence excerpt for previews.\n\
        5. Suggest a meta_title (≤60 chars) and meta_description (≤160 chars).\n\
        6. Identify 3–5 relevant tags.\n\n\
        When you're done, use the `create_post` tool to publish the draft."
    );

    Ok(vec![PromptMessage {
        role: PromptRole::User,
        content: PromptContent::Text(instruction),
    }])
}

async fn seo_review(ctx: &McpContext, args: &Value) -> Result<Vec<PromptMessage>, ErrorObject> {
    let slug = args
        .get("slug")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorObject::invalid_params("missing 'slug' argument"))?;

    // Fetch the actual post so the prompt has real data.
    let post = ctx
        .state
        .post_service
        .get(&ctx.auth, slug)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    let content_preview = if post.content.len() > 3000 {
        format!("{}…[truncated]", &post.content[..3000])
    } else {
        post.content.clone()
    };

    let instruction = format!(
        "You are an SEO and content quality auditor. Review the blog post below.\n\n\
        **Title:** {title}\n\
        **Slug:** {slug}\n\
        **Excerpt:** {excerpt}\n\
        **Meta Title:** {meta_title}\n\
        **Meta Description:** {meta_description}\n\n\
        **Content (Markdown):**\n{content}\n\n\
        Analyze the following and give actionable recommendations:\n\
        1. Title quality — is it compelling and keyword-rich? Is it under 60 characters?\n\
        2. Meta description — is it under 160 chars? Does it drive click-through?\n\
        3. Content structure — headings, paragraphs, readability (Flesch score).\n\
        4. Keyword usage — is there a clear primary keyword? Any stuffing?\n\
        5. Internal linking opportunities.\n\
        6. Image alt text suggestions (if applicable).\n\n\
        Rate each area out of 10 and provide an overall score with a prioritized action list.",
        title = post.title,
        slug = post.slug,
        excerpt = post.excerpt.as_deref().unwrap_or("(none)"),
        meta_title = post.meta_title.as_deref().unwrap_or("(none)"),
        meta_description = post.meta_description.as_deref().unwrap_or("(none)"),
        content = content_preview,
    );

    Ok(vec![PromptMessage {
        role: PromptRole::User,
        content: PromptContent::Text(instruction),
    }])
}

async fn summarize_latest(
    ctx: &McpContext,
    args: &Value,
) -> Result<Vec<PromptMessage>, ErrorObject> {
    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(20) as i64;

    let (posts, total) = ctx
        .state
        .post_service
        .list(&ctx.auth, 1, count, None, None, None)
        .await
        .map_err(|e| ErrorObject::internal_error(e.to_string()))?;

    if posts.is_empty() {
        return Ok(vec![PromptMessage {
            role: PromptRole::User,
            content: PromptContent::Text(
                "There are no blog posts yet. Create one using the create_post tool.".to_string(),
            ),
        }]);
    }

    let post_summaries: Vec<String> = posts
        .iter()
        .map(|p| {
            let status = format!("{:?}", p.status);
            format!(
                "- **{title}** (/{slug})\n  Excerpt: {excerpt}\n  Status: {status}",
                title = p.title,
                slug = p.slug,
                excerpt = p.excerpt.as_deref().unwrap_or("(none)"),
            )
        })
        .collect();

    let instruction = format!(
        "You are a content curator. Produce a digest of the latest {count} blog posts \
        (total in system: {total}).\n\n\
        **Recent posts:**\n{posts}\n\n\
        Write a concise digest that:\n\
        1. Summarizes the overall theme of recent posts.\n\
        2. Highlights the 2–3 most important posts with a 1-sentence summary each.\n\
        3. Suggests topics for follow-up posts based on what's trending.\n\n\
        Format it as a newsletter-ready blurb in Markdown.",
        count = count,
        total = total,
        posts = post_summaries.join("\n"),
    );

    Ok(vec![PromptMessage {
        role: PromptRole::User,
        content: PromptContent::Text(instruction),
    }])
}

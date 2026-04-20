//! SEO 优化插件（Component Model 版本）
//!
//! 零 unsafe 代码，直接操作类型化的 PostInput / String。
//! 不需要的 Hook 方法留空即可。

rust_blog_plugin_sdk::wit_bindgen::generate!({
    path: "../../plugins-protocol/wit/plugin.wit",
    world: "plugin-world",
});

use exports::rust_blog::plugin_protocol::plugin_hooks::{
    CommentInput, ContentEvent, Guest, PostInput, PostOutput,
};

struct SeoOptimizer;

impl Guest for SeoOptimizer {
    fn on_post_creating(input: PostInput) -> Option<PostInput> {
        let mut input = input;
        if input.excerpt.is_none() || input.excerpt.as_deref() == Some("") {
            let plain = strip_markdown(&input.content);
            input.excerpt = Some(truncate(&plain, 200));
        }
        Some(input)
    }

    fn on_post_updating(_input: PostInput) -> Option<PostInput> {
        None
    }
    fn on_comment_creating(_input: CommentInput) -> Option<CommentInput> {
        None
    }
    fn on_content_creating(_input: ContentEvent) -> Option<ContentEvent> {
        None
    }
    fn on_content_updating(_input: ContentEvent) -> Option<ContentEvent> {
        None
    }

    fn filter_html(input: String) -> Option<String> {
        let enhanced = inject_meta_tags(&input);
        Some(enhanced)
    }

    fn render_markdown(_input: String) -> Option<String> {
        None
    }
    fn on_post_created(_output: PostOutput) {}
    fn on_post_updated(_output: PostOutput) {}
    fn on_post_deleted(_id: String) {}
    fn on_comment_created(_input: CommentInput) {}
    fn on_content_created(_input: ContentEvent) {}
    fn on_content_updated(_input: ContentEvent) {}
    fn on_content_deleted(_content_type: String, _id: String) {}
    fn on_login(_user_id: String) {}
    fn on_cron_tick(_payload: Option<String>) {}
}

export!(SeoOptimizer);

fn strip_markdown(md: &str) -> String {
    let mut result = String::with_capacity(md.len());
    let mut in_code_block = false;
    for line in md.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        let cleaned = line
            .replace('#', "")
            .replace('*', "")
            .replace('_', "")
            .replace('`', "")
            .replace("![", "")
            .replace('[', "")
            .replace(']', "");
        if !cleaned.trim().is_empty() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(cleaned.trim());
        }
    }
    result
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

fn inject_meta_tags(html: &str) -> String {
    let meta = r#"<meta property="og:type" content="article">"#;
    html.replacen("<head>", &format!("<head>{meta}"), 1)
}

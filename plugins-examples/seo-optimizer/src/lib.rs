//! SEO 优化插件
//!
//! 自动为文章生成摘要（excerpt），如果作者未提供的话。

use rust_blog_plugin_sdk::*;

/// 创建文章前的过滤器：自动生成 excerpt
#[unsafe(no_mangle)]
pub extern "C" fn on_post_creating(ptr: i32, len: i32) -> i32 {
    let mut input: CreatePostInput = read_input(ptr, len);

    if input.excerpt.is_none() || input.excerpt.as_deref() == Some("") {
        let plain = strip_markdown(&input.content);
        input.excerpt = Some(truncate(&plain, 200));
    }

    write_output(&input)
}

/// Markdown 渲染过滤器：注入 OG 标签
#[unsafe(no_mangle)]
pub extern "C" fn filter_html(ptr: i32, len: i32) -> i32 {
    let html = read_string_input(ptr, len);
    let enhanced = inject_meta_tags(&html);
    write_string_output(&enhanced)
}

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

//! Markdown to HTML rendering pipeline.
//!
//! First converts Markdown text to HTML via **comrak**,
//! then sanitizes the generated HTML using **ammonia** to prevent XSS attacks.
//!
//! Code blocks (` ``` `) preserve language-identifier CSS classes for frontend JS highlighting libraries (e.g. highlight.js).

use ammonia::clean;
use comrak::{ComrakOptions, markdown_to_html};

/// Renders Markdown text to sanitized HTML.
///
/// 1. Uses comrak to convert Markdown to raw HTML (code blocks preserve `language-xxx` class).
/// 2. Uses ammonia to sanitize the HTML, removing dangerous tags and attributes.
#[must_use]
pub fn render_markdown(content: &str) -> String {
    let mut options = ComrakOptions::default();
    options.render.unsafe_ = true;
    let html = markdown_to_html(content, &options);
    clean(&html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_paragraph() {
        let html = render_markdown("hello **world**");
        assert!(html.contains("<strong>world</strong>"));
    }

    #[test]
    fn renders_code_block() {
        let input = "```rust\nfn main() {}\n```";
        let html = render_markdown(input);
        assert!(
            html.contains("<code>fn main()"),
            "expected code block in: {html}"
        );
    }

    #[test]
    fn sanitizes_script_tags() {
        let input = "hello <script>alert('xss')</script> world";
        let html = render_markdown(input);
        assert!(!html.contains("<script>"));
        assert!(html.contains("hello"));
    }

    #[test]
    fn renders_heading() {
        let html = render_markdown("# Title");
        assert!(html.contains("<h1>Title</h1>"));
    }

    #[test]
    fn renders_inline_code() {
        let html = render_markdown("use `cargo test`");
        assert!(html.contains("<code>cargo test</code>"));
    }
}

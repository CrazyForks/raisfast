//! Markdown 转 HTML 渲染管线。
//!
//! 先通过 **comrak** 将 Markdown 文本转换为 HTML，
//! 再通过 **ammonia** 对生成的 HTML 进行白名单过滤，防止 XSS 攻击。
//!
//! 代码块（` ``` `）保留语言标识 CSS class，由前端 JS 高亮库（如 highlight.js）渲染。

use ammonia::clean;
use comrak::{markdown_to_html, ComrakOptions};

/// 将 Markdown 文本渲染为经过安全过滤的 HTML。
///
/// 1. 使用 comrak 将 Markdown 转为原始 HTML（代码块保留 `language-xxx` class）。
/// 2. 使用 ammonia 对 HTML 进行消毒处理，移除危险标签和属性。
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

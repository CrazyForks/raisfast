//! Markdown 转 HTML 渲染管线。
//!
//! 先通过 **comrak** 将 Markdown 文本转换为 HTML（含 syntect 语法高亮），
//! 再通过 **ammonia** 对生成的 HTML 进行白名单过滤，防止 XSS 攻击。
//!
//! 代码块（` ``` `）自动获得语法高亮，支持通过语言标识指定语法（如 ` ```rust `）。
//! 使用 CSS class 模式（`SyntectAdapter::new(None)`），配合前端样式表渲染颜色。

use ammonia::clean;
use comrak::plugins::syntect::SyntectAdapter;
use comrak::{ComrakOptions, Plugins, RenderPlugins, markdown_to_html_with_plugins};

/// 将 Markdown 文本渲染为经过安全过滤的 HTML。
///
/// 1. 使用 comrak 将 Markdown 转为原始 HTML（含 syntect 代码高亮）。
/// 2. 使用 ammonia 对 HTML 进行消毒处理，移除危险标签和属性。
#[must_use]
pub fn render_markdown(content: &str) -> String {
    let mut options = ComrakOptions::default();
    options.render.unsafe_ = true;

    let adapter = SyntectAdapter::new(None);
    let render_plugins = RenderPlugins::builder()
        .codefence_syntax_highlighter(&adapter)
        .build();
    let plugins = Plugins::builder().render(render_plugins).build();

    let html = markdown_to_html_with_plugins(content, &options, &plugins);
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
    fn renders_code_block_with_css_classes() {
        let input = "```rust\nfn main() {}\n```";
        let html = render_markdown(input);
        assert!(
            html.contains("class=\"language-rust\"") || html.contains("<span"),
            "expected syntax highlighting in: {html}"
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

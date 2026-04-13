//! Markdown 转 HTML 渲染管线。
//!
//! 先通过 **comrak** 将 Markdown 文本转换为 HTML，
//! 再通过 **ammonia** 对生成的 HTML 进行白名单过滤，防止 XSS 攻击。

use ammonia::clean;
use comrak::markdown_to_html;

/// 将 Markdown 文本渲染为经过安全过滤的 HTML。
///
/// 1. 使用 comrak 将 Markdown 转为原始 HTML。
/// 2. 使用 ammonia 对 HTML 进行消毒处理，移除危险标签和属性。
pub fn render_markdown(content: &str) -> String {
    let html = markdown_to_html(content, &comrak::Options::default());
    clean(&html)
}

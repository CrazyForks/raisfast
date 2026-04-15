//! 国际化语言区域检测中间件
//!
//! 本模块为每个 HTTP 请求检测并绑定语言区域（locale），用于后续的
//! i18n 消息翻译（如错误提示）。通过 [`tokio::task_local!] 在请求生命周期内
//! 传递 locale，避免在 handler 签名中显式注入。
//!
//! # 语言检测优先级
//!
//! 1. URL 查询参数 `?lang=`（如 `?lang=zh-CN`）
//! 2. `Accept-Language` 请求头（遵循 RFC 7231 q 值权重）
//! 3. 默认值 `"en"`

use std::cmp::Ordering;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

// 当前请求的语言区域。
//
// 由 [`locale_middleware`] 在请求入口处设置，可通过 [`current_locale`] 读取。
// 使用 `tokio::task_local!` 实现，确保在同一个请求的异步调用链中
// locale 始终可用，而无需手动传参。
tokio::task_local! {
    static CURRENT_LOCALE: String;
}

/// 获取当前请求的语言区域。
///
/// 从 task-local 上下文中读取 locale；若在请求作用域外调用
/// （如后台任务、测试），则回退为默认值 `"en"`。
#[must_use]
pub fn current_locale() -> String {
    CURRENT_LOCALE
        .try_with(std::clone::Clone::clone)
        .unwrap_or_else(|_| "en".to_string())
}

/// 根据请求信息检测语言区域。
///
/// 检测优先级：
/// 1. `?lang=` 查询参数 — 若值在支持列表中则直接使用
/// 2. `Accept-Language` 请求头 — 按 q 值权重选择最优匹配
/// 3. 默认 `"en"`
pub fn detect_locale(req: &Request) -> String {
    if let Some(lang) = req.uri().query().and_then(|q| {
        q.split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| *k == "lang")
            .map(|(_, v)| v.to_string())
    }) {
        let lang = lang.to_lowercase();
        if ["zh-cn", "zh-tw", "zh", "en", "ja", "ko"].contains(&lang.as_str()) {
            return normalize_locale(&lang);
        }
    }

    req.headers()
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_accept_language)
        .unwrap_or_else(|| "en".to_string())
}

/// 解析 RFC 7231 `Accept-Language` 请求头。
///
/// 将头部值按逗号分隔为多个条目，解析每项的语言标签与可选的 `q` 权重值
/// （默认 `q=1.0`），返回权重最高的语言标签（经 [`normalize_locale`] 规范化后）。
///
/// # 示例
///
/// 输入 `"zh-CN,zh;q=0.9,en;q=0.8"` 将返回 `"zh-CN"`。
fn parse_accept_language(header: &str) -> Option<String> {
    header
        .split(',')
        .filter_map(|part| {
            let (lang, quality) = if let Some((l, q)) = part.trim().split_once(';') {
                let quality = q
                    .trim()
                    .strip_prefix("q=")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(1.0);
                (l.trim().to_lowercase(), quality)
            } else {
                (part.trim().to_lowercase(), 1.0)
            };
            if lang.is_empty() {
                None
            } else {
                Some((normalize_locale(&lang), quality))
            }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
        .map(|(lang, _)| lang)
}

/// 将语言标签规范化为项目的标准形式。
///
/// # 映射规则
///
/// | 输入 | 输出 |
/// |---|---|
/// | `zh`、`zh-cn`、`zh-hans` | `zh-CN` |
/// | `zh-tw`、`zh-hant` | `zh-TW` |
/// | `en`、`en-us`、`en-gb` | `en` |
/// | `ja` | `ja` |
/// | `ko` | `ko` |
/// | 其他 | 原样返回 |
fn normalize_locale(lang: &str) -> String {
    match lang {
        "zh" | "zh-cn" | "zh-hans" => "zh-CN".to_string(),
        "zh-tw" | "zh-hant" => "zh-TW".to_string(),
        "en" | "en-us" | "en-gb" => "en".to_string(),
        "ja" => "ja".to_string(),
        "ko" => "ko".to_string(),
        other => other.to_string(),
    }
}

/// Axum 语言区域检测中间件。
///
/// 对每个请求调用 [`detect_locale`] 确定语言，然后通过
/// [`CURRENT_LOCALE::scope`] 将其绑定到 task-local 上下文中，
/// 使后续的 handler 和 service 层可通过 [`current_locale`] 获取当前 locale。
pub async fn locale_middleware(req: Request, next: Next) -> Response {
    let locale = detect_locale(&req);
    rust_i18n::set_locale(&locale);
    CURRENT_LOCALE.scope(locale, next.run(req)).await
}

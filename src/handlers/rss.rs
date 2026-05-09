//! RSS 订阅源处理器
//!
//! 生成文章 RSS XML 订阅源，包含最近 20 篇已发布文章。

use axum::body::Body;
use axum::extract::State;
use axum::response::Response;

use crate::errors::app_error::AppResult;
use crate::middleware::locale::current_locale;
use crate::models::post;

/// RSS 订阅源
///
/// - **方法/路径：** `GET /rss`
/// - **认证：** 无需认证
/// - **说明：** 生成 RSS 2.0 XML 格式的订阅源，包含最近 20 篇已发布文章。
///   标题和描述通过 i18n 根据当前语言环境翻译。
/// - **返回：** `application/xml` 格式的 RSS 响应
pub async fn feed(State(state): State<crate::AppState>) -> AppResult<Response> {
    let locale = current_locale();
    rust_i18n::set_locale(&locale);
    let posts = post::find_published_joined(&state.pool, 1, 20, None, None, None, None)
        .await?
        .0;

    let base_url = &state.config.base_url;

    let mut items = Vec::new();
    for p in posts {
        items.push(
            rss::ItemBuilder::default()
                .title(Some(p.title.clone()))
                .link(Some(format!("{}/posts/{}", base_url, p.slug)))
                .description(p.excerpt.clone())
                .pub_date(p.published_at.map(|t| t.to_rfc3339()))
                .build(),
        );
    }

    let rss_title = rust_i18n::t!("rss.title");
    let rss_desc = rust_i18n::t!("rss.description");

    let channel = rss::ChannelBuilder::default()
        .title(rss_title)
        .link(base_url.clone())
        .description(rss_desc)
        .items(items)
        .build();

    let body = channel.to_string();

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/xml")
        .body(Body::from(body))
        .unwrap_or_else(|_| {
            let error_msg = rust_i18n::t!("rss.error");
            Response::new(Body::from(format!("<error>{error_msg}</error>")))
        }))
}

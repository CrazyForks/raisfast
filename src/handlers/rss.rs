//! RSS feed handler
//!
//! Generates an RSS XML feed containing the 20 most recently published posts.

use axum::body::Body;
use axum::extract::State;
use axum::response::Response;

use crate::errors::app_error::AppResult;
use crate::middleware::locale::current_locale;

/// RSS feed
///
/// - **Method/Path:** `GET /rss`
/// - **Auth:** Not required
/// - **Description:** Generates an RSS 2.0 XML feed containing the 20 most recently published posts.
///   Titles and descriptions are translated via i18n based on the current locale.
/// - **Returns:** RSS response in `application/xml` format
#[utoipa::path(get, path = "/rss", tag = "rss",
    responses((status = 200, description = "RSS XML feed"))
)]
pub async fn feed(State(state): State<crate::AppState>) -> AppResult<Response> {
    let locale = current_locale();
    rust_i18n::set_locale(&locale);
    let posts = state.post_service.list_recent_published(20, None).await?;

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

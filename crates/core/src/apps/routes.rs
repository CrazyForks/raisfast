//! App Bundle admin route registration (§11 MVP subset).

use crate::AppState;

/// Routes under `/api/v1/admin/apps`. Package uploads raise the body limit to
/// the configured max upload size (the global 2 MiB cap would reject them).
pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<AppState> {
    let upload_limit = tower_http::limit::RequestBodyLimitLayer::new(config.max_upload_size);
    let r = axum::Router::new();
    let r = crate::reg_route!(
        r,
        registry,
        false,
        "/admin/apps",
        get,
        crate::apps::admin::list_apps,
        "apps",
        "admin/apps"
    );
    let r = crate::reg_route!(
        r,
        registry,
        false,
        "/admin/apps/install-preview",
        post,
        crate::apps::admin::install_preview,
        "apps",
        "admin/apps"
    );
    let r = crate::reg_route!(
        r,
        registry,
        false,
        "/admin/apps/install",
        post,
        crate::apps::admin::install,
        "apps",
        "admin/apps"
    )
    .layer(upload_limit);
    let r = crate::reg_route!(
        r,
        registry,
        false,
        "/admin/apps/{app_id}",
        get,
        crate::apps::admin::get_app,
        "apps",
        "admin/apps"
    );
    let r = crate::reg_route!(
        r,
        registry,
        false,
        "/admin/apps/{app_id}/enable",
        post,
        crate::apps::admin::enable_app,
        "apps",
        "admin/apps"
    );
    let r = crate::reg_route!(
        r,
        registry,
        false,
        "/admin/apps/{app_id}/disable",
        post,
        crate::apps::admin::disable_app,
        "apps",
        "admin/apps"
    );
    crate::reg_route!(
        r,
        registry,
        false,
        "/admin/apps/{app_id}/uninstall",
        post,
        crate::apps::admin::uninstall_app,
        "apps",
        "admin/apps"
    )
}

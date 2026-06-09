//! HTTP server: route assembly, middleware, startup, and graceful shutdown.

mod openapi;

use std::sync::Arc;
use std::time::Duration;

use crate::AppState;
use crate::admin_spa;
use crate::cache::MemoryCache;
use crate::config::app::AppConfig;
use crate::constants::DEFAULT_TENANT;
use crate::handlers::payment as h_payment;
use crate::handlers::{
    api_token, auth, category, comment, cron, health, media, options, page, plugin, post,
    product_category, rbac, reusable_block, rss, setup, sse, stats, tag, tenant, user, wallet, ws,
};
use crate::middleware::locale::locale_middleware;
use crate::middleware::metrics;
use crate::middleware::rate_limit::{RateLimiterSet, global_rate_limit};
use crate::workflow;
use axum::Extension;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::Level;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteInfo {
    pub method: String,
    pub path: String,
    pub source: String,
    pub source_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct RouteRegistry {
    routes: Vec<RouteInfo>,
}

impl RouteRegistry {
    pub fn record(&mut self, method: &str, path: &str, source: &str, source_name: &str) {
        self.routes.push(RouteInfo {
            method: method.to_string(),
            path: path.to_string(),
            source: source.to_string(),
            source_name: source_name.to_string(),
        });
    }

    pub fn into_vec(self) -> Vec<RouteInfo> {
        self.routes
    }
}

/// Build the CORS middleware.
fn build_cors(config: &AppConfig) -> CorsLayer {
    match &config.cors_origins {
        Some(origins) => {
            let allow: Vec<HeaderValue> = origins
                .split(',')
                .filter_map(|o: &str| o.trim().parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(allow)
                .allow_methods(Any)
                .allow_headers(Any)
        }
        None => CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    }
}

async fn build_app(
    config: &AppConfig,
    limiters: RateLimiterSet,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<axum::Router> {
    let upload_dir = config.upload_dir.clone();
    let static_dir = config.static_dir.clone();
    let max_upload = config.max_upload_size;

    let mut registry = RouteRegistry::default();

    let mut state = crate::build_app_state(config, shutdown_rx).await?;
    let pool = state.pool.clone();
    let eventbus = state.eventbus.clone();
    let worker_pool = pool.clone();

    if config.worker_enabled {
        let cache_for_workers: Arc<dyn crate::cache::CacheStore> = Arc::new(MemoryCache::new());
        spawn_workers(
            worker_pool,
            &eventbus,
            config,
            state.plugins.clone(),
            state.search.clone(),
            cache_for_workers,
        )
        .await;
    }

    let cors = build_cors(config);
    let mut api_v1 = axum::Router::new();

    api_v1 = api_v1
        .merge(setup::routes(&mut registry, config))
        .merge(auth::routes(&mut registry, config))
        .merge(crate::handlers::oauth::routes(&mut registry, config))
        .merge(api_token::routes(&mut registry, config))
        .merge(user::routes(&mut registry, config));

    if config.builtins.blog {
        api_v1 = api_v1
            .merge(category::routes(&mut registry, config))
            .merge(product_category::routes(&mut registry, config))
            .merge(tag::routes(&mut registry, config))
            .merge(post::routes(&mut registry, config))
            .merge(post::admin_routes(&mut registry, config))
            .merge(comment::routes(&mut registry, config));
    }

    if config.builtins.ecommerce {
        api_v1 = api_v1
            .merge(crate::handlers::product::routes(&mut registry, config))
            .merge(crate::handlers::order::routes(&mut registry, config))
            .merge(crate::handlers::cart::routes(&mut registry, config))
            .merge(crate::handlers::product_variant::routes(
                &mut registry,
                config,
            ))
            .merge(crate::handlers::product_comment::routes(
                &mut registry,
                config,
            ))
            .merge(crate::handlers::user_address::routes(&mut registry, config))
            .merge(crate::handlers::coupon::routes(&mut registry, config))
            .merge(crate::handlers::shipping_template::routes(
                &mut registry,
                config,
            ))
            .merge(crate::handlers::currencies::routes(&mut registry, config));
    }

    if config.builtins.payment {
        api_v1 = api_v1.merge(h_payment::routes(&mut registry, config));
    }

    if config.builtins.wallet {
        api_v1 = api_v1.merge(wallet::routes(&mut registry, config));
    }

    if config.builtins.pages {
        api_v1 = api_v1
            .merge(page::routes(&mut registry, config))
            .merge(reusable_block::routes(&mut registry, config));
    }

    if config.builtins.media {
        api_v1 = api_v1.merge(media::routes(max_upload, &mut registry, config));
    }

    api_v1 = api_v1.merge(sse::routes(&mut registry, config));

    if config.websocket_enabled {
        tracing::info!("WebSocket enabled at /api/v1/ws");
        api_v1 = api_v1.merge(ws::routes(&mut registry, config));
    }

    if config.graphql_enabled {
        tracing::info!("GraphQL enabled at /api/v1/graphql");
        api_v1 = api_v1.merge(crate::graphql::handler::routes(&mut registry, config));
    }

    api_v1 = api_v1
        .merge(plugin::routes(&mut registry, config))
        .merge(cron::routes(&mut registry, config))
        .merge(rbac::routes(&mut registry, config))
        .merge(stats::routes(&mut registry, config))
        .merge(options::routes(&mut registry, config))
        .merge(tenant::routes(&mut registry, config))
        .merge(crate::handlers::audit::routes(&mut registry, config))
        .merge(crate::webhook::handler::routes(&mut registry, config))
        .merge(crate::content_type::handler::routes(&mut registry, config));

    if config.builtins.workflow {
        api_v1 = api_v1.merge(workflow::handler::routes(&mut registry, config));
    }

    api_v1 = api_v1
        .layer(from_fn(global_rate_limit))
        .layer(Extension(limiters.clone()))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));

    api_v1 = crate::content_type::handler::register_content_routes(
        api_v1,
        &state.content_type_registry,
        &state.protocol_registry,
        config,
    );

    let restful = config.api_restful;
    for ct in state.content_type_registry.all() {
        let plural = &ct.plural;
        let name = &ct.singular;
        if ct.is_single() {
            registry.record(
                "GET",
                &format!("{}/{}", crate::constants::CMS_PREFIX, name),
                "content_type",
                name,
            );
            if restful {
                registry.record(
                    "PUT",
                    &format!("{}/{}", crate::constants::CMS_PREFIX, name),
                    "content_type",
                    name,
                );
            } else {
                registry.record(
                    "POST",
                    &format!("{}/{}/update", crate::constants::CMS_PREFIX, name),
                    "content_type",
                    name,
                );
            }
            registry.record(
                "GET",
                &format!("{}/{}", crate::constants::CMS_ADMIN_PREFIX, name),
                "content_type",
                name,
            );
        } else if restful {
            for (method, suffix) in [
                ("GET", ""),
                ("POST", ""),
                ("GET", "/{id}"),
                ("PUT", "/{id}"),
                ("DELETE", "/{id}"),
            ] {
                registry.record(
                    method,
                    &format!("{}/{}{}", crate::constants::CMS_PREFIX, plural, suffix),
                    "content_type",
                    name,
                );
            }
            registry.record(
                "GET",
                &format!("{}/{}", crate::constants::CMS_ADMIN_PREFIX, plural),
                "content_type",
                name,
            );
            registry.record(
                "GET",
                &format!("{}/{}/{{id}}", crate::constants::CMS_ADMIN_PREFIX, plural),
                "content_type",
                name,
            );
        } else {
            registry.record(
                "GET",
                &format!("{}/{}", crate::constants::CMS_PREFIX, plural),
                "content_type",
                name,
            );
            registry.record(
                "POST",
                &format!("{}/{}/create", crate::constants::CMS_PREFIX, plural),
                "content_type",
                name,
            );
            registry.record(
                "GET",
                &format!("{}/{}/{{id}}", crate::constants::CMS_PREFIX, plural),
                "content_type",
                name,
            );
            registry.record(
                "POST",
                &format!("{}/{}/{{id}}/update", crate::constants::CMS_PREFIX, plural),
                "content_type",
                name,
            );
            registry.record(
                "POST",
                &format!("{}/{}/{{id}}/delete", crate::constants::CMS_PREFIX, plural),
                "content_type",
                name,
            );
            registry.record(
                "GET",
                &format!("{}/{}", crate::constants::CMS_ADMIN_PREFIX, plural),
                "content_type",
                name,
            );
            registry.record(
                "GET",
                &format!("{}/{}/{{id}}", crate::constants::CMS_ADMIN_PREFIX, plural),
                "content_type",
                name,
            );
        }
    }

    api_v1 = api_v1
        .route("/routes", get(list_routes))
        .route("/health", get(health::health));

    registry.record(
        "GET",
        &format!("{}/health", crate::constants::API_PREFIX),
        "system",
        "health",
    );

    {
        let plugin_routes = state.plugins.all_plugin_routes().await;
        for (method, path, ext_id) in &plugin_routes {
            registry.record(method, path, "plugin", ext_id);
        }
    }

    registry.record(
        "GET",
        &format!("{}/routes", crate::constants::API_PREFIX),
        "system",
        "system",
    );

    let routes_vec = registry.into_vec();
    state.route_registry = Arc::new(routes_vec);

    let app = axum::Router::new()
        .route("/health", get(health::health))
        .route("/healthz", get(health::liveness))
        .route("/readyz", get(health::readiness))
        .route("/metrics", get(metrics::metrics_endpoint))
        .route("/feed.xml", get(rss::feed))
        .route("/api/v1/info", get(server_info_handler))
        .nest(crate::constants::API_PREFIX, api_v1)
        .nest_service("/uploads", ServeDir::new(&upload_dir))
        .nest_service("/static", ServeDir::new(&static_dir))
        .nest(
            "/admin",
            axum::Router::new().fallback(admin_spa::serve_admin),
        )
        .nest(
            "/assets",
            axum::Router::new().fallback(admin_spa::serve_admin_asset),
        )
        .fallback(handle_plugin_route)
        .layer(Extension(limiters))
        .layer(from_fn(powered_by_middleware))
        .layer(from_fn(locale_middleware))
        .layer(from_fn(metrics::track_metrics))
        .layer(from_fn(crate::middleware::request_id::inject_request_id))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    let method = request.method().as_str();
                    let uri = request.uri().path();
                    tracing::span!(
                        Level::DEBUG,
                        "request",
                        method,
                        uri,
                        request_id = tracing::field::Empty
                    )
                })
                .on_request(|_request: &axum::extract::Request, _span: &tracing::Span| {})
                .on_response(
                    |response: &axum::response::Response,
                     latency: Duration,
                     _span: &tracing::Span| {
                        tracing::debug!(
                            status = %response.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "request completed"
                        );
                    },
                ),
        )
        .layer(cors)
        .layer(from_fn(
            crate::middleware::security_headers::security_headers,
        ))
        .layer(from_fn_with_state(
            state.clone(),
            crate::middleware::aop_http::aop_http_layer,
        ))
        .layer(CompressionLayer::new())
        .layer(from_fn_with_state(
            state.clone(),
            crate::middleware::tenant::subdomain_tenant_resolver,
        ))
        .with_state(state);

    let app = app.route("/api/docs/openapi.json", get(openapi::serve_openapi_json));

    #[cfg(feature = "openapi")]
    let app = app
        .route("/api/docs", get(openapi::serve_scalar_ui))
        .route("/api/docs/", get(openapi::serve_scalar_ui));

    Ok(app)
}

/// Start the HTTP server, listening for requests until a shutdown signal is received.
pub async fn start(config: &AppConfig) -> anyhow::Result<()> {
    metrics::init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        env = %config.env,
        "starting raisfast server"
    );
    let tz = crate::utils::tz::parse_tz_or_utc(&config.timezone);
    tracing::info!("site timezone: {}", tz);
    crate::utils::tz::set_site_tz(tz);
    let addr = format!("{}:{}", config.host, config.port);
    let limiters = RateLimiterSet::from_config(config);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let cleanup_limiters = limiters.clone();
    let mut cleanup_shutdown = shutdown_rx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    cleanup_limiters.global.cleanup_expired().await;
                    cleanup_limiters.register.cleanup_expired().await;
                    cleanup_limiters.login.cleanup_expired().await;
                    cleanup_limiters.comment.cleanup_expired().await;
                    cleanup_limiters.api_token.cleanup_expired().await;
                }
                _ = cleanup_shutdown.changed() => {
                    tracing::info!("rate limiter cleanup task shutting down");
                    break;
                }
            }
        }
    });

    let app = build_app(config, limiters, shutdown_rx).await?;

    match (&config.tls_cert_path, &config.tls_key_path) {
        (Some(_cert), Some(_key)) => {
            #[cfg(feature = "tls")]
            {
                use axum_server::tls_rustls::RustlsConfig;
                let tls_config = RustlsConfig::from_pem_file(_cert, _key).await?;
                tracing::info!("server listening on https://{}", addr);
                println!("server listening on https://{}", addr);
                let socket_addr: std::net::SocketAddr = addr.parse()?;
                axum_server::bind_rustls(socket_addr, tls_config)
                    .serve(app.into_make_service())
                    .await?;
            }
            #[cfg(not(feature = "tls"))]
            {
                tracing::warn!(
                    "TLS_CERT_PATH and TLS_KEY_PATH set but 'tls' feature not enabled. \
                      Falling back to HTTP."
                );
                tracing::info!(
                    "server listening on http://{} (pid={})",
                    addr,
                    std::process::id()
                );
                println!("server listening on http://{}", addr);
                let start = std::time::Instant::now();
                let listener = TcpListener::bind(&addr).await?;
                tracing::info!(
                    startup_ms = start.elapsed().as_millis() as u64,
                    "server ready to accept connections"
                );
                axum::serve(listener, app.into_make_service())
                    .with_graceful_shutdown(shutdown_signal(shutdown_tx.clone()))
                    .await?;

                tracing::info!("server shutdown complete");
            }
        }
        _ => {
            tracing::info!("server listening on http://{}", addr);
            println!("server listening on http://{}", addr);
            let listener = TcpListener::bind(&addr).await?;
            axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(shutdown_signal(shutdown_tx))
                .await?;
        }
    }

    Ok(())
}

/// Listen for ctrl+c (SIGINT) and SIGTERM to perform graceful shutdown.
///
/// On signal, notifies `shutdown_tx` so background tasks can clean up.
async fn shutdown_signal(shutdown_tx: tokio::sync::watch::Sender<bool>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .unwrap_or_else(|e| tracing::error!("ctrl+c handler failed: {e}"));
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(e) => tracing::error!("SIGTERM handler failed: {e}"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("received SIGINT (ctrl+c), starting graceful shutdown");
        },
        _ = terminate => {
            tracing::info!("received SIGTERM, starting graceful shutdown");
        },
    }

    let _ = shutdown_tx.send(true);
}

/// Plugin route fallback.
///
/// When no axum route matches, attempt to dispatch to plugin declarative routes from `manifest.routes`.
/// If no plugin handles the request, returns 404.
async fn handle_plugin_route(
    Extension(limiters): Extension<crate::middleware::rate_limit::RateLimiterSet>,
    auth: crate::middleware::auth::AuthUser,
    State(state): State<AppState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use serde_json::json;

    let ip = crate::middleware::rate_limit::extract_client_ip(&req);
    if !limiters.global.check(&ip).await {
        return crate::middleware::rate_limit::rate_limited_response();
    }
    if let Some(prefix) = crate::middleware::rate_limit::extract_api_token_prefix(&req)
        && !limiters.api_token.check(&format!("token:{prefix}")).await
    {
        return crate::middleware::rate_limit::rate_limited_response();
    }

    let content_length = req
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if let Some(len) = content_length
        && len > 2 * 1024 * 1024
    {
        return (
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(json!({
                "code": 41300,
                "message": "request body too large",
                "data": null
            })),
        )
            .into_response();
    }

    let path = {
        let mut s = req.uri().path().to_string();
        if let Some(q) = req.uri().query() {
            s.push('?');
            s.push_str(q);
        }
        s
    };
    let method = req.method().to_string();

    let headers_json: serde_json::Value = {
        let mut map = serde_json::Map::new();
        for (key, value) in req.headers() {
            if let Ok(v) = value.to_str() {
                map.insert(key.to_string(), serde_json::Value::String(v.to_string()));
            }
        }
        serde_json::Value::Object(map)
    };

    let body_str = if method == "GET" || method == "HEAD" {
        None
    } else {
        axum::body::to_bytes(req.into_body(), 1024 * 1024)
            .await
            .ok()
            .and_then(|b| String::from_utf8(b.to_vec()).ok())
    };

    let result = state
        .plugins
        .dispatch_route(
            &path,
            &method,
            body_str.as_deref(),
            Some(&headers_json),
            &auth,
        )
        .await;

    match result {
        Some(response) => response,
        None => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(json!({
                "code": 40400,
                "message": "not found",
                "data": null
            })),
        )
            .into_response(),
    }
}

/// Spawn EventBus background subscriber to forward business events to the plugin system.
/// Spawn audit log subscriber to write all business events to the `audit_log` table.
pub fn spawn_audit_subscriber(
    eventbus: crate::eventbus::EventBus,
    audit: Arc<crate::services::audit::AuditService>,
    tenant_service: Arc<crate::services::tenant::TenantService>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        let default_tenant: &str = DEFAULT_TENANT;
        let _ = tenant_service;
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(event) => {
                    let Some(info) = event.audit_info() else {
                        continue;
                    };

                    if let Err(e) = audit
                        .log(
                            default_tenant,
                            info.actor_id,
                            None,
                            &info.action,
                            &info.subject,
                            Some(&info.subject_id),
                            info.detail.as_deref(),
                            None,
                            None,
                        )
                        .await
                    {
                        tracing::warn!(action = %info.action, subject = %info.subject, error = %e, "failed to write audit log");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("audit subscriber lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("audit subscriber shutting down");
                    break;
                }
            }
        }
    });
}

/// Derive webhook event type from event metadata.
///
/// Format: `{singular_table}.{action}` where action is extracted from `display_name()`
/// by stripping the PascalCase table prefix (e.g., `PostCreated` → `post.created`).
fn webhook_event_type(event: &crate::eventbus::Event) -> Option<String> {
    let table = event.table()?;
    let singular = table.strip_suffix('s').unwrap_or(table);
    let display = event.display_name();
    let prefix: String = singular
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    let action = display.strip_prefix(&prefix)?;
    let action = action[..1].to_ascii_lowercase() + &action[1..];
    Some(format!("{}.{}", singular, action))
}

/// Spawn webhook event delivery subscriber
pub fn spawn_webhook_subscriber(
    eventbus: crate::eventbus::EventBus,
    webhook_service: Arc<crate::webhook::WebhookService>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|e| {
            tracing::error!("webhook http client init failed: {e}; using default client");
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        });

    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(event) => {
                            let event_type = match webhook_event_type(event.as_ref()) {
                                Some(t) => t,
                                None => continue,
                            };

                    let payload_value = serde_json::to_value(event.as_ref()).unwrap_or_default();
                    let timestamp = crate::utils::tz::now_utc();
                    let webhook_payload = crate::webhook::model::WebhookPayload {
                        event: event_type.clone(),
                        data: payload_value,
                        timestamp,
                    };
                    let body = match serde_json::to_vec(&webhook_payload) {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!("webhook payload serialize error: {e}");
                            continue;
                        }
                    };

                    let subs = match webhook_service.find_enabled(Some(DEFAULT_TENANT)).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("webhook find_enabled error: {e}");
                            continue;
                        }
                    };

                    for sub in subs {
                        let events: Vec<String> =
                            serde_json::from_str(&sub.events).unwrap_or_default();
                        if !events.iter().any(|e| {
                            e == &event_type || e == "*" || event_type.starts_with(&format!("{e}."))
                        }) {
                            continue;
                        }

                        let signature = crate::webhook::service::WebhookService::sign_payload(
                            &sub.secret,
                            &body,
                        );
                        let url = sub.url.clone();
                        let body_clone = body.clone();
                        let event_type = event_type.clone();
                        let client = client.clone();
                        tokio::spawn(async move {
                            let result = client
                                .post(&url)
                                .header("Content-Type", "application/json")
                                .header("X-Webhook-Signature", format!("sha256={signature}"))
                                .header("X-Webhook-Event", event_type)
                                .body(body_clone)
                                .send()
                                .await;
                            match result {
                                Ok(resp) => {
                                    tracing::debug!(
                                        url = %url,
                                        status = %resp.status(),
                                        "webhook delivered"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        url = %url,
                                        error = %e,
                                        "webhook delivery failed"
                                    );
                                }
                            }
                        });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("webhook subscriber lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("webhook subscriber shutting down");
                    break;
                }
            }
        }
    });
}

/// Spawn the Worker subsystem (CronScheduler + JobEnqueuer + WorkerRunner)
async fn spawn_workers(
    pool: crate::db::Pool,
    eventbus: &crate::eventbus::EventBus,
    config: &AppConfig,
    plugins: Arc<crate::plugins::PluginManager>,
    search: Arc<dyn crate::search::SearchEngine>,
    cache: Arc<dyn crate::cache::CacheStore>,
) {
    use crate::worker::{
        CronScheduler, DefaultJobQueue, JobEnqueuer, JobHandlerRegistry, PluginCronDispatcher,
        WorkerRunner, seed_defaults,
    };

    let queue = Arc::new(DefaultJobQueue::new(pool.clone()));

    if let Err(e) = async {
        crate::db::connection::ensure_schema(&pool).await?;
        if config.cron_seed_enabled {
            seed_defaults(&pool, &config.cron_schedules).await?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await
    {
        tracing::warn!("worker migration/seed error: {e}");
    }

    let mut registry = JobHandlerRegistry::new();
    crate::worker::handlers::register_all(
        &mut registry,
        pool.clone(),
        Arc::new(config.clone()),
        search,
        cache,
        crate::notifier::build_email_sender(config),
        crate::notifier::build_sms_sender(config),
    );

    let cron = CronScheduler::new(
        pool,
        queue.clone(),
        Duration::from_millis(config.worker_cron_tick_ms),
    );
    cron.spawn();

    JobEnqueuer::spawn(eventbus, queue.clone());

    let runner = WorkerRunner::new(
        queue,
        Arc::new(registry),
        Duration::from_millis(config.worker_poll_interval_ms),
        config.worker_batch_size,
    )
    .with_plugin_dispatcher(Arc::new(PluginCronDispatcher::new(plugins)));
    runner.spawn(config.worker_concurrency);

    tracing::info!(
        "worker system started: concurrency={}, poll={}ms, batch={}",
        config.worker_concurrency,
        config.worker_poll_interval_ms,
        config.worker_batch_size,
    );
}

async fn list_routes(State(state): State<AppState>) -> impl IntoResponse {
    use serde_json::json;

    let mut routes: Vec<RouteInfo> = state.route_registry.as_ref().clone();
    routes.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.path.cmp(&b.path)));

    axum::Json(json!({
        "code": 0,
        "data": routes,
        "message": "ok"
    }))
}

async fn powered_by_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    if let (Ok(name), Ok(value)) = (
        axum::http::header::HeaderName::from_bytes(b"x-powered-by"),
        axum::http::HeaderValue::from_str(&crate::_brand()),
    ) {
        response.headers_mut().insert(name, value);
    }
    response
}

async fn server_info_handler(State(state): State<AppState>) -> impl IntoResponse {
    use serde_json::json;

    let api_style = if state.config.api_restful {
        "restful"
    } else {
        "simple"
    };
    axum::Json(json!({
        "code": 0,
        "data": {
            "name": crate::_brand(),
            "version": env!("CARGO_PKG_VERSION"),
            "api_style": api_style,
        }
    }))
}

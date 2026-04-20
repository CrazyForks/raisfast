//! HTTP 服务器：路由组装、中间件、启动与优雅关闭。

mod openapi;

use std::sync::Arc;
use std::time::Duration;

use crate::AppState;
use crate::cache::MemoryCache;
use crate::config::app::AppConfig;
use crate::content_type::ContentTypeRegistry;
use crate::db::connection::init_pool;
use crate::handlers::{
    api_token, auth, category, comment, cron, health, media, options, plugin, post, rbac, rss, sse,
    stats, tag, tenant, user, workflow,
};
use crate::middleware::locale::locale_middleware;
use crate::middleware::metrics;
use crate::middleware::rate_limit::{
    RateLimiterSet, comment_rate_limit, global_rate_limit, login_rate_limit, register_rate_limit,
};
use crate::repositories::{
    CachedPostRepository, OptionsRepository, PostRepository, RbacRepository,
    SqlxCategoryRepository, SqlxCommentRepository, SqlxMediaRepository, SqlxOptionsRepository,
    SqlxPostRepository, SqlxRbacRepository, SqlxRefreshTokenRepository, SqlxTagRepository,
    SqlxTenantRepository, SqlxUserRepository, TenantRepository,
};
use crate::search::{NoopSearchEngine, SearchEngine};
use crate::services::options::OptionsService;
use crate::services::rbac::RbacService;
use crate::services::tenant::TenantService;
use axum::Extension;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::middleware::from_fn;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post as http_post, put};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::Level;

/// 构建搜索引擎实例
fn build_search_engine(config: &AppConfig) -> Arc<dyn SearchEngine> {
    match config.search_engine.as_str() {
        #[cfg(feature = "search-tantivy")]
        "tantivy" => match crate::search::TantivyEngine::open(&config.search_index_dir) {
            Ok(engine) => {
                tracing::info!(
                    "search engine: tantivy (index: {})",
                    config.search_index_dir
                );
                Arc::new(engine)
            }
            Err(e) => {
                tracing::error!("failed to open tantivy index: {e}, falling back to noop");
                Arc::new(NoopSearchEngine)
            }
        },
        _ => {
            tracing::info!("search engine: none (LIKE fallback)");
            Arc::new(NoopSearchEngine)
        }
    }
}

/// 构建 CORS 中间件。
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

/// 组装完整的应用路由（含数据库连接池初始化）。
async fn build_app(config: &AppConfig, limiters: RateLimiterSet) -> anyhow::Result<axum::Router> {
    let upload_dir = config.upload_dir.clone();
    let static_dir = config.static_dir.clone();
    let max_upload = config.max_upload_size;
    let pool = init_pool(&config.database_url, config.db_pool_size).await?;

    let eventbus = crate::eventbus::EventBus::new(256);

    let worker_pool = pool.clone();

    let sqlx_repo = SqlxPostRepository::new(pool.clone());
    let cache: std::sync::Arc<dyn crate::cache::CacheStore> =
        std::sync::Arc::new(MemoryCache::new());
    let post_repo: Arc<dyn PostRepository> =
        Arc::new(CachedPostRepository::new(sqlx_repo, cache, None));

    let user_repo: Arc<dyn crate::repositories::UserRepository> =
        Arc::new(SqlxUserRepository::new(pool.clone()));
    let category_repo: Arc<dyn crate::repositories::CategoryRepository> =
        Arc::new(SqlxCategoryRepository::new(pool.clone()));
    let tag_repo: Arc<dyn crate::repositories::TagRepository> =
        Arc::new(SqlxTagRepository::new(pool.clone()));
    let comment_repo: Arc<dyn crate::repositories::CommentRepository> =
        Arc::new(SqlxCommentRepository::new(pool.clone()));
    let media_repo: Arc<dyn crate::repositories::MediaRepository> =
        Arc::new(SqlxMediaRepository::new(pool.clone()));
    let refresh_token_repo: Arc<dyn crate::repositories::RefreshTokenRepository> =
        Arc::new(SqlxRefreshTokenRepository::new(pool.clone()));

    let search: Arc<dyn SearchEngine> = build_search_engine(config);

    let ct_registry = Arc::new(ContentTypeRegistry::new());

    let plugin_manager = crate::plugins::PluginManager::new_empty(
        Arc::new(config.clone()),
        crate::plugins::PluginManagerOptions {
            pool: Some(pool.clone()),
        },
    )
    .await;

    let extension_manager = crate::extension::manager::ExtensionManager::new(
        ct_registry.clone(),
        plugin_manager.clone(),
        pool.clone(),
        config,
    )
    .await;

    let options_repo: Arc<dyn OptionsRepository> =
        Arc::new(SqlxOptionsRepository::new(pool.clone()));
    let options_service = Arc::new(OptionsService::new(options_repo).await);

    let rbac_repo: Arc<dyn RbacRepository> = Arc::new(SqlxRbacRepository::new(pool.clone()));
    let rbac_service = Arc::new(RbacService::new(rbac_repo));

    let tenant_repo: Arc<dyn TenantRepository> = Arc::new(SqlxTenantRepository::new(pool.clone()));
    let tenant_service = Arc::new(TenantService::new(tenant_repo));
    let audit_service = Arc::new(crate::audit::AuditService::new(pool.clone()));
    let webhook_service = Arc::new(crate::webhook::WebhookService::new(pool.clone()));

    let extension_service = Arc::new(crate::extension::service::ExtensionService::new(
        pool.clone(),
    ));

    let storage = crate::storage::create_storage(config)?;

    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config.clone()),
        jwt_decoding_key: jsonwebtoken::DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        plugins: plugin_manager,
        eventbus: eventbus.clone(),
        post_repo,
        user_repo,
        category_repo,
        tag_repo,
        comment_repo,
        media_repo,
        refresh_token_repo,
        search,
        content_type_registry: ct_registry,
        options: options_service,
        rbac: rbac_service,
        tenant: tenant_service,
        audit: audit_service,
        webhook: webhook_service.clone(),
        workflow: Arc::new(crate::services::workflow::WorkflowService::new(
            pool.clone(),
        )),
        extension_manager,
        extension_service,
        storage,
        cms_cache: Arc::new(dashmap::DashMap::new()),
    };

    spawn_event_subscriber(eventbus.clone(), state.plugins.clone());
    spawn_audit_subscriber(eventbus.clone(), state.audit.clone(), state.tenant.clone());
    spawn_webhook_subscriber(eventbus.clone(), state.webhook.clone());

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

    let api_v1 = axum::Router::new()
        .route(
            "/auth/register",
            http_post(auth::register).layer(from_fn(register_rate_limit)),
        )
        .route(
            "/auth/login",
            http_post(auth::login).layer(from_fn(login_rate_limit)),
        )
        .route("/auth/refresh", http_post(auth::refresh))
        .route("/auth/logout", http_post(auth::logout))
        .route("/tokens", get(api_token::list).post(api_token::create))
        .route("/tokens/{id}", delete(api_token::delete))
        .route("/users/me", get(user::get_me).put(user::update_me))
        .route("/users/me/password", put(user::change_password))
        .route("/users/{id}", get(user::get_user))
        .route("/users/{id}/role", put(user::update_role))
        .route("/users", get(user::list_users))
        .route("/categories", get(category::list).post(category::create))
        .route(
            "/categories/{id}",
            put(category::update).delete(category::delete),
        )
        .route("/tags", get(tag::list).post(tag::create))
        .route("/tags/{id}", delete(tag::delete))
        .route("/posts", get(post::list).post(post::create))
        .route(
            "/posts/{slug}",
            get(post::get).put(post::update).delete(post::delete),
        )
        .route("/posts/{slug}/comments", get(comment::list))
        .route(
            "/posts/{slug}/comments",
            http_post(comment::create_guest).layer(from_fn(comment_rate_limit)),
        )
        .route("/posts/{slug}/comments/authed", http_post(comment::create))
        .route("/comments/{id}", delete(comment::delete))
        .route("/comments/{id}/status", put(comment::update_status))
        .route("/comments", get(comment::list_all))
        .route(
            "/media/upload",
            http_post(media::upload).layer(RequestBodyLimitLayer::new(max_upload)),
        )
        .route("/media", get(media::list))
        .route("/media/stats", get(media::stats))
        .route("/media/{id}", delete(media::delete))
        .route("/events", get(sse::subscribe))
        .route("/admin/posts", get(post::admin_list))
        .route("/admin/posts/{slug}", get(post::admin_get))
        .route("/admin/plugins", get(plugin::list))
        .route(
            "/admin/plugins/{id}",
            get(plugin::get).delete(plugin::remove),
        )
        .route("/admin/plugins/{id}/enable", http_post(plugin::enable))
        .route("/admin/plugins/{id}/disable", http_post(plugin::disable))
        .route("/admin/plugins/{id}/reload", http_post(plugin::reload))
        .route("/admin/crons", get(cron::list).post(cron::create))
        .route(
            "/admin/crons/{id}",
            get(cron::get).put(cron::update).delete(cron::delete),
        )
        .route("/admin/crons/{id}/toggle", http_post(cron::toggle))
        .route("/admin/crons/logs", get(cron::logs))
        .route("/admin/crons/logs/cleanup", http_post(cron::cleanup_logs))
        .route(
            "/admin/rbac/roles",
            get(rbac::list_roles).post(rbac::create_role),
        )
        .route(
            "/admin/rbac/roles/{id}",
            put(rbac::update_role).delete(rbac::delete_role),
        )
        .route(
            "/admin/rbac/roles/{id}/permissions",
            get(rbac::get_permissions).put(rbac::set_permissions),
        )
        .route("/admin/stats", get(stats::overview))
        .route("/admin/stats/content/{table}", get(stats::content_stats))
        .route("/admin/stats/trends", get(stats::trends))
        .route("/options/public", get(options::get_public_options))
        .route(
            "/admin/options",
            get(options::list_options).put(options::update_options),
        )
        .route(
            "/admin/options/{key}",
            get(options::get_option)
                .put(options::set_option)
                .delete(options::delete_option),
        )
        .route(
            "/admin/tenants",
            get(tenant::list_tenants).post(tenant::create_tenant),
        )
        .route(
            "/admin/tenants/{id}",
            get(tenant::get_tenant)
                .put(tenant::update_tenant)
                .delete(tenant::delete_tenant),
        )
        .route("/admin/audit", get(crate::audit::handler::list))
        .route("/admin/audit/{id}", get(crate::audit::handler::get))
        .route(
            "/admin/webhooks",
            get(crate::webhook::handler::list).post(crate::webhook::handler::create),
        )
        .route(
            "/admin/webhooks/{id}",
            get(crate::webhook::handler::get)
                .put(crate::webhook::handler::update)
                .delete(crate::webhook::handler::delete),
        )
        .route(
            "/admin/content-types",
            get(crate::content_type::handler::list_schemas)
                .post(crate::content_type::handler::create_schema),
        )
        .route(
            "/admin/content-types/{singular}",
            get(crate::content_type::handler::get_schema)
                .put(crate::content_type::handler::update_schema)
                .delete(crate::content_type::handler::delete_schema),
        )
        .route("/admin/extensions", get(crate::extension::handler::list))
        .route(
            "/admin/extensions/{id}",
            get(crate::extension::handler::get).delete(crate::extension::handler::uninstall),
        )
        .route(
            "/admin/extensions/{id}/enable",
            http_post(crate::extension::handler::enable),
        )
        .route(
            "/admin/extensions/{id}/disable",
            http_post(crate::extension::handler::disable),
        )
        .route(
            "/admin/workflows",
            get(workflow::list).post(workflow::create),
        )
        .route(
            "/admin/workflows/{id}",
            get(workflow::get).delete(workflow::delete),
        )
        .route("/admin/workflows/{id}/start", http_post(workflow::start))
        .route("/admin/workflows/instances", get(workflow::list_instances))
        .route(
            "/admin/workflows/instances/{id}",
            get(workflow::get_instance),
        )
        .route(
            "/admin/workflows/instances/{id}/execute",
            http_post(workflow::execute_step),
        )
        .route(
            "/admin/workflows/instances/{id}/cancel",
            http_post(workflow::cancel_instance),
        )
        .route(
            "/admin/workflows/instances/{id}/logs",
            get(workflow::get_step_logs),
        )
        .layer(from_fn(global_rate_limit))
        .layer(Extension(limiters))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));

    let api_v1 =
        crate::content_type::handler::register_content_routes(api_v1, &state.content_type_registry);

    let api_v1 = api_v1
        .route(
            "/cms/{*path}",
            axum::routing::any(crate::content_type::handler::dynamic_cms_handler),
        )
        .route(
            "/admin/cms/{*path}",
            axum::routing::any(crate::content_type::handler::dynamic_admin_cms_handler),
        );

    let app = axum::Router::new()
        .route("/health", get(health::health))
        .route("/healthz", get(health::liveness))
        .route("/readyz", get(health::readiness))
        .route("/metrics", get(metrics::metrics_endpoint))
        .route("/feed.xml", get(rss::feed))
        .nest("/api/v1", api_v1)
        .nest_service("/uploads", ServeDir::new(&upload_dir))
        .nest_service("/static", ServeDir::new(&static_dir))
        .fallback(handle_plugin_route)
        .layer(from_fn(locale_middleware))
        .layer(from_fn(metrics::track_metrics))
        .layer(from_fn(crate::middleware::request_id::inject_request_id))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    let method = request.method().as_str();
                    let uri = request.uri().path();
                    let version = match request.version() {
                        axum::http::Version::HTTP_09 => "0.9",
                        axum::http::Version::HTTP_10 => "1.0",
                        axum::http::Version::HTTP_11 => "1.1",
                        axum::http::Version::HTTP_2 => "2.0",
                        axum::http::Version::HTTP_3 => "3.0",
                        _ => "unknown",
                    };
                    tracing::span!(
                        Level::INFO,
                        "request",
                        method,
                        uri,
                        version,
                        request_id = tracing::field::Empty
                    )
                })
                .on_request(|request: &axum::extract::Request, _span: &tracing::Span| {
                    tracing::info!(
                        method = %request.method(),
                        path = %request.uri().path(),
                        "--> request start"
                    );
                })
                .on_response(
                    |response: &axum::response::Response,
                     latency: Duration,
                     _span: &tracing::Span| {
                        tracing::info!(
                            status = %response.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "<-- request done"
                        );
                    },
                ),
        )
        .layer(cors)
        .with_state(state);

    let app = app.route("/api/docs/openapi.json", get(openapi::serve_openapi_json));

    #[cfg(feature = "openapi")]
    let app = app
        .route("/api/docs", get(openapi::redirect_to_swagger))
        .route("/api/docs/", get(openapi::redirect_to_swagger));

    Ok(app)
}

/// 启动 HTTP 服务器，监听请求直到收到关闭信号。
pub async fn start(config: &AppConfig) -> anyhow::Result<()> {
    metrics::init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        env = %config.env,
        "starting rust-blog server"
    );
    let tz = crate::utils::tz::parse_tz_or_utc(&config.timezone);
    tracing::info!("site timezone: {}", tz);
    crate::utils::tz::set_site_tz(tz);
    let addr = format!("{}:{}", config.host, config.port);
    let limiters = RateLimiterSet::from_config(config);

    let cleanup_limiters = limiters.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            cleanup_limiters.global.cleanup_expired().await;
            cleanup_limiters.register.cleanup_expired().await;
            cleanup_limiters.login.cleanup_expired().await;
            cleanup_limiters.comment.cleanup_expired().await;
            cleanup_limiters.api_token.cleanup_expired().await;
        }
    });

    let app = build_app(config, limiters).await?;

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
                    .with_graceful_shutdown(shutdown_signal())
                    .await?;

                tracing::info!("server shutdown complete");
            }
        }
        _ => {
            tracing::info!("server listening on http://{}", addr);
            println!("server listening on http://{}", addr);
            let listener = TcpListener::bind(&addr).await?;
            axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
    }

    Ok(())
}

/// 监听 ctrl+c (SIGINT) 和 SIGTERM 实现优雅关闭。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl+c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
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
}

/// 插件路由 fallback。
///
/// 当 axum 路由未匹配时，尝试分发给插件的 `manifest.routes` 声明式路由。
/// 若所有插件均未处理，返回 404。
async fn handle_plugin_route(
    auth: crate::middleware::auth::OptionalAuth,
    State(state): State<AppState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use serde_json::json;

    let path = req.uri().path().to_string();
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

    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024).await;
    let body_str = body_bytes
        .ok()
        .and_then(|b| String::from_utf8(b.to_vec()).ok());

    let result = state
        .plugins
        .dispatch_route(
            &path,
            &method,
            body_str.as_deref(),
            Some(&headers_json),
            auth.0.as_ref(),
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

/// 启动 EventBus 后台订阅者，将业务事件转发给插件系统。
fn spawn_event_subscriber(
    eventbus: crate::eventbus::EventBus,
    plugins: Arc<crate::plugins::PluginManager>,
) {
    use crate::eventbus::Event;
    use crate::plugins::HookPoint;

    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => match event.as_ref() {
                    Event::PostCreated { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins.dispatch_action(HookPoint::PostCreated, &json).await;
                    }
                    Event::PostUpdated { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins.dispatch_action(HookPoint::PostUpdated, &json).await;
                    }
                    Event::PostDeleted { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins.dispatch_action(HookPoint::PostDeleted, &json).await;
                    }
                    Event::CommentCreated { .. } => {
                        let json = serde_json::to_value(event.as_ref()).unwrap_or_default();
                        plugins
                            .dispatch_action(HookPoint::CommentCreated, &json)
                            .await;
                    }
                    _ => {}
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("eventbus subscriber lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });
}

/// 启动审计日志订阅者，将所有业务事件写入 `audit_log` 表。
fn spawn_audit_subscriber(
    eventbus: crate::eventbus::EventBus,
    audit: Arc<crate::audit::AuditService>,
    tenant_service: Arc<crate::services::tenant::TenantService>,
) {
    use crate::eventbus::Event;

    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        let default_tenant: &str = "default";
        let _ = tenant_service;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let (action, subject, subject_id, actor_id, detail): (
                        &str,
                        &str,
                        String,
                        Option<String>,
                        Option<String>,
                    ) = match event.as_ref() {
                        Event::PostCreated {
                            id,
                            title,
                            author_id,
                            ..
                        } => (
                            "create",
                            "post",
                            id.clone(),
                            Some(author_id.clone()),
                            Some(format!("title={title}")),
                        ),
                        Event::PostUpdated { id, slug } => (
                            "update",
                            "post",
                            id.clone(),
                            None,
                            Some(format!("slug={slug}")),
                        ),
                        Event::PostDeleted { id, slug } => (
                            "delete",
                            "post",
                            id.clone(),
                            None,
                            Some(format!("slug={slug}")),
                        ),
                        Event::CommentCreated {
                            id, author_name, ..
                        } => (
                            "create",
                            "comment",
                            id.clone(),
                            None,
                            Some(format!("author={author_name}")),
                        ),
                        Event::CommentDeleted { id } => {
                            ("delete", "comment", id.clone(), None, None)
                        }
                        Event::ContentCreated {
                            content_type, id, ..
                        } => ("create", content_type.as_str(), id.clone(), None, None),
                        Event::ContentUpdated { content_type, id } => {
                            ("update", content_type.as_str(), id.clone(), None, None)
                        }
                        Event::ContentDeleted { content_type, id } => {
                            ("delete", content_type.as_str(), id.clone(), None, None)
                        }
                        Event::UserRegistered { id, username, .. } => (
                            "register",
                            "user",
                            id.clone(),
                            None,
                            Some(format!("username={username}")),
                        ),
                        Event::UserLoggedIn { id, success } => (
                            "login",
                            "user",
                            id.clone(),
                            Some(id.clone()),
                            Some(format!("success={success}")),
                        ),
                        Event::MediaUploaded {
                            id,
                            filename,
                            uploader_id,
                        } => (
                            "upload",
                            "media",
                            id.clone(),
                            Some(uploader_id.clone()),
                            Some(format!("filename={filename}")),
                        ),
                        Event::MediaDeleted { id } => ("delete", "media", id.clone(), None, None),
                        _ => continue,
                    };

                    if let Err(e) = audit
                        .log(
                            default_tenant,
                            actor_id.as_deref(),
                            None,
                            action,
                            subject,
                            Some(&subject_id),
                            detail.as_deref(),
                            None,
                            None,
                        )
                        .await
                    {
                        tracing::warn!(%action, %subject, error = %e, "failed to write audit log");
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
    });
}

/// 启动 Webhook 事件投递订阅者
fn spawn_webhook_subscriber(
    eventbus: crate::eventbus::EventBus,
    webhook_service: Arc<crate::webhook::WebhookService>,
) {
    use crate::eventbus::Event;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|e| {
            tracing::error!("webhook http client init failed: {e}");
            panic!("webhook client failure");
        });

    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let event_type = match event.as_ref() {
                        Event::PostCreated { .. } => "post.created",
                        Event::PostUpdated { .. } => "post.updated",
                        Event::PostDeleted { .. } => "post.deleted",
                        Event::CommentCreated { .. } => "comment.created",
                        Event::CommentDeleted { .. } => "comment.deleted",
                        Event::ContentCreated { .. } => {
                            continue;
                        }
                        Event::ContentUpdated { .. } => {
                            continue;
                        }
                        Event::ContentDeleted { .. } => {
                            continue;
                        }
                        Event::UserRegistered { .. } => "user.registered",
                        Event::UserLoggedIn { .. } => "user.loggedIn",
                        Event::MediaUploaded { .. } => "media.uploaded",
                        Event::MediaDeleted { .. } => "media.deleted",
                        _ => continue,
                    };

                    let payload_value = serde_json::to_value(event.as_ref()).unwrap_or_default();
                    let timestamp = chrono::Utc::now().to_rfc3339();
                    let webhook_payload = crate::webhook::model::WebhookPayload {
                        event: event_type.to_string(),
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

                    let subs = match webhook_service.find_enabled("default").await {
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
                            e == event_type || e == "*" || event_type.starts_with(&format!("{e}."))
                        }) {
                            continue;
                        }

                        let signature = crate::webhook::service::WebhookService::sign_payload(
                            &sub.secret,
                            &body,
                        );
                        let url = sub.url.clone();
                        let body_clone = body.clone();
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
    });
}

/// 启动 Worker 子系统（CronScheduler + JobEnqueuer + WorkerRunner）
async fn spawn_workers(
    pool: crate::db::Pool,
    eventbus: &crate::eventbus::EventBus,
    config: &AppConfig,
    plugins: Arc<crate::plugins::PluginManager>,
    search: Arc<dyn crate::search::SearchEngine>,
    cache: Arc<dyn crate::cache::CacheStore>,
) {
    use crate::worker::{
        CronScheduler, JobEnqueuer, JobHandlerRegistry, PluginCronDispatcher, SqliteJobQueue,
        WorkerRunner, seed_defaults,
    };

    let queue = Arc::new(SqliteJobQueue::new(pool.clone()));

    if let Err(e) = async {
        sqlx::query(include_str!("../migrations/006_jobs.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../migrations/007_cron_schedules.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../migrations/008_cron_execution_log.sql"))
            .execute(&pool)
            .await?;
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
    )
    .with_plugin_dispatcher(Arc::new(PluginCronDispatcher::new(plugins)));
    runner.spawn(config.worker_concurrency);

    tracing::info!(
        "worker system started: concurrency={}, poll={}ms",
        config.worker_concurrency,
        config.worker_poll_interval_ms,
    );
}

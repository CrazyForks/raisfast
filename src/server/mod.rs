//! HTTP 服务器：路由组装、中间件、启动与优雅关闭。

use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::middleware::from_fn;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post as http_post, put};
use rust_blog::AppState;
use rust_blog::cache::MemoryCache;
use rust_blog::config::app::AppConfig;
use rust_blog::db::connection::init_pool;
use rust_blog::handlers::{
    auth, category, comment, cron, health, media, plugin, post, rss, sse, tag, user,
};
use rust_blog::middleware::locale::locale_middleware;
use rust_blog::middleware::rate_limit::{
    RateLimiterSet, comment_rate_limit, global_rate_limit, login_rate_limit, register_rate_limit,
};
use rust_blog::repositories::{
    CachedPostRepository, PostRepository, SqlxCategoryRepository, SqlxCommentRepository,
    SqlxMediaRepository, SqlxPostRepository, SqlxRefreshTokenRepository, SqlxTagRepository,
    SqlxUserRepository,
};
use rust_blog::search::{NoopSearchEngine, SearchEngine};
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
        "tantivy" => {
            match rust_blog::search::TantivyEngine::open(&config.search_index_dir) {
                Ok(engine) => {
                    tracing::info!("search engine: tantivy (index: {})", config.search_index_dir);
                    Arc::new(engine)
                }
                Err(e) => {
                    tracing::error!("failed to open tantivy index: {e}, falling back to noop");
                    Arc::new(NoopSearchEngine)
                }
            }
        }
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

    let eventbus = rust_blog::eventbus::EventBus::new(256);

    let worker_pool = pool.clone();

    let sqlx_repo = SqlxPostRepository::new(pool.clone());
    let cache: std::sync::Arc<dyn rust_blog::cache::CacheStore> =
        std::sync::Arc::new(MemoryCache::new());
    let post_repo: Arc<dyn PostRepository> =
        Arc::new(CachedPostRepository::new(sqlx_repo, cache, None));

    let user_repo: Arc<dyn rust_blog::repositories::UserRepository> =
        Arc::new(SqlxUserRepository::new(pool.clone()));
    let category_repo: Arc<dyn rust_blog::repositories::CategoryRepository> =
        Arc::new(SqlxCategoryRepository::new(pool.clone()));
    let tag_repo: Arc<dyn rust_blog::repositories::TagRepository> =
        Arc::new(SqlxTagRepository::new(pool.clone()));
    let comment_repo: Arc<dyn rust_blog::repositories::CommentRepository> =
        Arc::new(SqlxCommentRepository::new(pool.clone()));
    let media_repo: Arc<dyn rust_blog::repositories::MediaRepository> =
        Arc::new(SqlxMediaRepository::new(pool.clone()));
    let refresh_token_repo: Arc<dyn rust_blog::repositories::RefreshTokenRepository> =
        Arc::new(SqlxRefreshTokenRepository::new(pool.clone()));

    let search: Arc<dyn SearchEngine> = build_search_engine(config);

    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config.clone()),
        plugins: rust_blog::plugins::PluginManager::new_with_options(
            Arc::new(config.clone()),
            rust_blog::plugins::PluginManagerOptions { pool: Some(pool) },
        )
        .await,
        eventbus: eventbus.clone(),
        post_repo,
        user_repo,
        category_repo,
        tag_repo,
        comment_repo,
        media_repo,
        refresh_token_repo,
        search,
    };

    spawn_event_subscriber(eventbus.clone(), state.plugins.clone());

    if config.worker_enabled {
        spawn_workers(worker_pool, &eventbus, config, state.plugins.clone(), state.search.clone()).await;
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
        .layer(from_fn(global_rate_limit))
        .layer(Extension(limiters))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));

    let app = axum::Router::new()
        .route("/health", get(health::health))
        .route("/feed.xml", get(rss::feed))
        .nest("/api/v1", api_v1)
        .nest_service("/uploads", ServeDir::new(&upload_dir))
        .nest_service("/static", ServeDir::new(&static_dir))
        .fallback(handle_plugin_route)
        .layer(from_fn(locale_middleware))
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
                    tracing::span!(Level::INFO, "request", method, uri, version)
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

    Ok(app)
}

/// 启动 HTTP 服务器，监听请求直到收到关闭信号。
pub async fn start(config: &AppConfig) -> anyhow::Result<()> {
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
        }
    });

    let app = build_app(config, limiters).await?;
    let listener = TcpListener::bind(&addr).await?;

    tracing::info!("server listening on {}", addr);
    println!("server listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
        _ = ctrl_c => { tracing::info!("received ctrl+c"); },
        _ = terminate => { tracing::info!("received SIGTERM"); },
    }
}

/// 插件路由 fallback。
///
/// 当 axum 路由未匹配时，尝试分发给插件的 `handle_route` Hook。
/// 若所有插件均未处理，返回 404。
async fn handle_plugin_route(
    State(state): State<AppState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use serde_json::json;

    let path = req.uri().path().to_string();
    let method = req.method().to_string();

    let result = state.plugins.dispatch_route(&path, &method).await;

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
    eventbus: rust_blog::eventbus::EventBus,
    plugins: Arc<rust_blog::plugins::PluginManager>,
) {
    use rust_blog::eventbus::Event;
    use rust_blog::plugins::HookPoint;

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

/// 启动 Worker 子系统（CronScheduler + JobEnqueuer + WorkerRunner）
async fn spawn_workers(
    pool: rust_blog::db::Pool,
    eventbus: &rust_blog::eventbus::EventBus,
    config: &AppConfig,
    plugins: Arc<rust_blog::plugins::PluginManager>,
    search: Arc<dyn rust_blog::search::SearchEngine>,
) {
    use rust_blog::worker::{
        CronScheduler, JobEnqueuer, JobHandlerRegistry, PluginCronDispatcher, SqliteJobQueue,
        WorkerRunner, seed_defaults,
    };
    use std::sync::Arc;
    use std::time::Duration;

    let queue = Arc::new(SqliteJobQueue::new(pool.clone()));

    if let Err(e) = async {
        sqlx::query(include_str!("../../migrations/006_jobs.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../../migrations/007_cron_schedules.sql"))
            .execute(&pool)
            .await?;
        sqlx::query(include_str!("../../migrations/008_cron_execution_log.sql"))
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
    rust_blog::worker::handlers::register_all(
        &mut registry,
        pool.clone(),
        Arc::new(config.clone()),
        search,
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

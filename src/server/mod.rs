//! HTTP 服务器：路由组装、中间件、启动与优雅关闭。

use std::sync::Arc;

use axum::Extension;
use axum::http::HeaderValue;
use axum::middleware::from_fn;
use axum::routing::{delete, get, post as http_post, put};
use rust_blog::AppState;
use rust_blog::config::app::AppConfig;
use rust_blog::db::connection::init_pool;
use rust_blog::handlers::{auth, category, comment, health, media, post, rss, tag, user};
use rust_blog::middleware::locale::locale_middleware;
use rust_blog::middleware::rate_limit::{
    RateLimiterSet, comment_rate_limit, global_rate_limit, login_rate_limit, register_rate_limit,
};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

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
    let max_upload = config.max_upload_size;
    let pool = init_pool(&config.database_url, config.db_pool_size).await?;

    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
    };

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
        .route(
            "/posts/{slug}/comments",
            get(comment::list)
                .post(comment::create_guest)
                .layer(from_fn(comment_rate_limit)),
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
        .layer(from_fn(global_rate_limit))
        .layer(Extension(limiters))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));

    let app = axum::Router::new()
        .route("/health", get(health::health))
        .route("/feed.xml", get(rss::feed))
        .nest("/api/v1", api_v1)
        .nest_service("/uploads", ServeDir::new(&upload_dir))
        .layer(from_fn(locale_middleware))
        .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()))
        .layer(cors)
        .with_state(state);

    Ok(app)
}

/// 启动 HTTP 服务器，监听请求直到收到关闭信号。
pub async fn start(config: &AppConfig) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let limiters = RateLimiterSet::new_default();

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

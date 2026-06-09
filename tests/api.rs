//! API 集成测试
//!
//! 覆盖所有 31 个 API 端点。使用 axum::Router + 内存 SQLite 数据库，
//! 通过 tower::ServiceExt::oneshot 发送请求，验证响应状态码和 JSON 结构。
//!
//! # 运行方式
//!
//! ```bash
//! cargo test
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware::from_fn;
use axum::routing::{delete, get, post as http_post, put};
use http_body_util::BodyExt;
use raisfast::AppState;
use raisfast::DbDriver;
use raisfast::config::app::AppConfig;
use raisfast::handlers::{
    api_token as h_token, auth as h_auth, cart as h_cart, category as h_cat, comment as h_cmt,
    cron as h_cron, health as h_health, media as h_media, options as h_options, order as h_order,
    page as h_page, payment as h_payment, plugin as h_plugin, post as h_post, product as h_product,
    product_category as h_product_category, product_variant as h_product_variant, rbac as h_rbac,
    reusable_block as h_block, rss as h_rss, sse as h_sse, stats as h_stats, tag as h_tag,
    tenant as h_tenant, user as h_user, user_address as h_user_address, wallet as h_wallet,
};
use raisfast::middleware::locale::locale_middleware;
use raisfast::middleware::rate_limit::{
    RateLimiterSet, comment_rate_limit, global_rate_limit, login_rate_limit,
    payment_callback_rate_limit, register_rate_limit,
};
use raisfast::plugins::PluginManager;
use raisfast::search::NoopSearchEngine;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::limit::RequestBodyLimitLayer;

// ── helpers ──────────────────────────────────────────────────────

pub(crate) fn test_config() -> AppConfig {
    let mut cfg = AppConfig::test_defaults();
    cfg.upload_dir = std::env::temp_dir()
        .join("hello-axum-test-uploads")
        .to_string_lossy()
        .into();
    cfg.base_url = "http://localhost:9000".into();
    let mut key_bytes = [0u8; 32];
    getrandom::getrandom(&mut key_bytes).unwrap();
    cfg.app_key = Some(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        key_bytes,
    ));
    cfg
}

pub(crate) async fn test_pool() -> raisfast::db::Pool {
    #[cfg(feature = "db-sqlite")]
    {
        let pool = raisfast::db::Pool::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(raisfast::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }
}

pub(crate) async fn test_pool_with_tenants() -> raisfast::db::Pool {
    #[cfg(feature = "db-sqlite")]
    {
        test_pool().await
    }
}

pub(crate) async fn test_app() -> (axum::Router, AppState) {
    build_test_app(test_pool().await).await
}

pub(crate) async fn test_app_with_tenants() -> (axum::Router, AppState) {
    build_test_app(test_pool_with_tenants().await).await
}

async fn build_test_app(pool: raisfast::db::Pool) -> (axum::Router, AppState) {
    let config = Arc::new(test_config());
    let state = AppState {
        pool: pool.clone(),
        config: config.clone(),
        jwt_decoding_key: jsonwebtoken::DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        plugins: PluginManager::new(config.clone()).await,
        eventbus: raisfast::eventbus::EventBus::new(256),
        post_service: {
            Arc::new(raisfast::services::post::PostServiceImpl::new(
                Arc::new(pool.clone()),
                Arc::new(raisfast::aspects::engine::AspectEngine::new()),
                Arc::new(NoopSearchEngine),
            ))
        },
        page_service: Arc::new(raisfast::services::page::PageServiceImpl::new(
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
            Arc::new(pool.clone()),
        )),
        category_service: Arc::new(raisfast::services::category::CategoryServiceImpl::new(
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
            Arc::new(pool.clone()),
        )),
        tag_service: Arc::new(raisfast::services::tag::TagServiceImpl::new(
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
            Arc::new(pool.clone()),
        )),
        comment_service: Arc::new(raisfast::services::comment::CommentServiceImpl::new(
            Arc::new(pool.clone()),
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
        )),
        wallet_service: Arc::new(raisfast::services::wallet::WalletServiceImpl::new(
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
            Arc::new(pool.clone()),
        )),
        product_category_service: Arc::new(
            raisfast::services::product_category::ProductCategoryServiceImpl::new(
                Arc::new(raisfast::aspects::engine::AspectEngine::new()),
                Arc::new(pool.clone()),
            ),
        ),
        product_service: Arc::new(raisfast::services::product::ProductServiceImpl::new(
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
            Arc::new(pool.clone()),
            Arc::new(
                raisfast::services::options::OptionsService::new(Arc::new(pool.clone()), false)
                    .await,
            ),
        )),
        order_service: Arc::new(raisfast::services::order::OrderServiceImpl::new(
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
            Arc::new(pool.clone()),
            Arc::new(
                raisfast::services::options::OptionsService::new(Arc::new(pool.clone()), false)
                    .await,
            ),
        )),
        cart_service: Arc::new(raisfast::services::cart::CartServiceImpl::new(Arc::new(
            pool.clone(),
        ))),
        product_variant_service: Arc::new(
            raisfast::services::product_variant::ProductVariantServiceImpl::new(Arc::new(
                pool.clone(),
            )),
        ),
        product_comment_service: Arc::new(
            raisfast::services::product_comment::ProductCommentServiceImpl::new(Arc::new(
                pool.clone(),
            )),
        ),
        coupon_service: Arc::new(raisfast::services::coupon::CouponServiceImpl::new(
            Arc::new(pool.clone()),
        )),
        shipping_template_service: Arc::new(
            raisfast::services::shipping_template::ShippingTemplateServiceImpl::new(Arc::new(
                pool.clone(),
            )),
        ),
        user_address_service: Arc::new(
            raisfast::services::user_address::UserAddressServiceImpl::new(Arc::new(pool.clone())),
        ),
        payment_service: Arc::new(raisfast::services::payment::PaymentServiceImpl::new(
            config.clone(),
            Arc::new(raisfast::aspects::engine::AspectEngine::new()),
            Arc::new(pool.clone()),
        )),
        user_service: Arc::new(raisfast::services::user::UserServiceImpl::new(Arc::new(
            pool.clone(),
        ))),
        search: Arc::new(NoopSearchEngine),
        content_type_registry: Arc::new(raisfast::content_type::ContentTypeRegistry::new()),
        aspect_engine: {
            let engine = raisfast::aspects::engine::AspectEngine::new();
            let mut reg = raisfast::protocols::ProtocolRegistry::new();
            reg.register(raisfast::protocols::ownable::OwnableProtocol);
            reg.register(raisfast::protocols::timestampable::TimestampableProtocol);
            let reg = Arc::new(reg);
            reg.register_aspects_into(&engine);
            Arc::new(engine)
        },
        protocol_registry: Arc::new({
            let mut reg = raisfast::protocols::ProtocolRegistry::new();
            reg.register(raisfast::protocols::ownable::OwnableProtocol);
            reg.register(raisfast::protocols::timestampable::TimestampableProtocol);
            reg
        }),
        options: Arc::new(
            raisfast::services::options::OptionsService::new(Arc::new(pool.clone()), false).await,
        ),
        rbac: Arc::new(raisfast::services::rbac::RbacService::new(Arc::new(
            pool.clone(),
        ))),
        tenant: Arc::new(raisfast::services::tenant::TenantService::new(Arc::new(
            pool.clone(),
        ))),
        audit: Arc::new(raisfast::services::audit::AuditService::new(pool.clone())),
        webhook: Arc::new(raisfast::webhook::WebhookService::new(pool.clone())),
        workflow: Arc::new(raisfast::workflow::WorkflowService::new(pool.clone())),
        storage: raisfast::storage::create_storage(&config).expect("failed to create storage"),
        cache: Arc::new(raisfast::cache::MemoryCache::new()),
        cms_cache: Arc::new(dashmap::DashMap::new()),
        oauth_registry: Arc::new(raisfast::oauth::OAuthProviderRegistry::default()),
        email_sender: raisfast::notifier::build_email_sender(&config),
        sms_sender: raisfast::notifier::build_sms_sender(&config),
        route_registry: Arc::new(Vec::new()),
        services: raisfast::app::ServiceRegistry::new(),
    };
    let max_upload = state.config.max_upload_size;

    let api_v1 = axum::Router::new()
        .route(
            "/auth/register",
            http_post(h_auth::register).layer(from_fn(register_rate_limit)),
        )
        .route(
            "/auth/login",
            http_post(h_auth::login).layer(from_fn(login_rate_limit)),
        )
        .route("/auth/refresh", http_post(h_auth::refresh))
        .route("/auth/logout", http_post(h_auth::logout))
        .route("/tokens", get(h_token::list).post(h_token::create))
        .route("/tokens/{id}", delete(h_token::delete))
        .route("/users/me", get(h_user::get_me).put(h_user::update_me))
        .route("/users/me/password", put(h_user::change_password))
        .route("/users/{id}", get(h_user::get_user))
        .route("/users/{id}/role", put(h_user::update_role))
        .route("/users", get(h_user::list_users))
        .route("/categories", get(h_cat::list).post(h_cat::create))
        .route("/categories/{id}", put(h_cat::update).delete(h_cat::delete))
        .route("/tags", get(h_tag::list).post(h_tag::create))
        .route("/tags/{id}", delete(h_tag::delete))
        .route("/posts", get(h_post::list).post(h_post::create))
        .route(
            "/posts/{slug}",
            get(h_post::get).put(h_post::update).delete(h_post::delete),
        )
        .route(
            "/posts/{slug}/comments",
            get(h_cmt::list)
                .post(h_cmt::create_guest)
                .layer(from_fn(comment_rate_limit)),
        )
        .route("/posts/{slug}/comments/authed", http_post(h_cmt::create))
        .route("/comments/{id}", delete(h_cmt::delete))
        .route("/comments/{id}/status", put(h_cmt::update_status))
        .route(
            "/media/upload",
            http_post(h_media::upload).layer(RequestBodyLimitLayer::new(max_upload)),
        )
        .route("/media", get(h_media::list))
        .route("/media/{id}", delete(h_media::delete))
        .route("/events", get(h_sse::subscribe))
        .route("/admin/crons", get(h_cron::list).post(h_cron::create))
        .route(
            "/admin/crons/{id}",
            get(h_cron::get).put(h_cron::update).delete(h_cron::delete),
        )
        .route("/admin/crons/{id}/toggle", http_post(h_cron::toggle))
        .route("/admin/crons/logs", get(h_cron::logs))
        .route("/admin/crons/logs/cleanup", http_post(h_cron::cleanup_logs))
        .route("/admin/plugins", get(h_plugin::list))
        .route(
            "/admin/plugins/{id}",
            get(h_plugin::get).delete(h_plugin::remove),
        )
        .route("/admin/plugins/{id}/enable", http_post(h_plugin::enable))
        .route("/admin/plugins/{id}/disable", http_post(h_plugin::disable))
        .route("/admin/plugins/{id}/reload", http_post(h_plugin::reload))
        .route(
            "/admin/rbac/roles",
            get(h_rbac::list_roles).post(h_rbac::create_role),
        )
        .route(
            "/admin/rbac/roles/{id}",
            put(h_rbac::update_role).delete(h_rbac::delete_role),
        )
        .route(
            "/admin/rbac/roles/{id}/permissions",
            get(h_rbac::get_permissions).put(h_rbac::set_permissions),
        )
        .route("/admin/stats", get(h_stats::overview))
        .route("/admin/stats/content/{table}", get(h_stats::content_stats))
        .route("/admin/stats/trends", get(h_stats::trends))
        .route("/options/public", get(h_options::get_public_options))
        .route(
            "/admin/options",
            get(h_options::list_options).put(h_options::update_options),
        )
        .route(
            "/admin/options/{key}",
            get(h_options::get_option)
                .put(h_options::set_option)
                .delete(h_options::delete_option),
        )
        .route(
            "/admin/tenants",
            get(h_tenant::list_tenants).post(h_tenant::create_tenant),
        )
        .route(
            "/admin/tenants/{id}",
            get(h_tenant::get_tenant)
                .put(h_tenant::update_tenant)
                .delete(h_tenant::delete_tenant),
        )
        .route("/admin/audit", get(raisfast::handlers::audit::list))
        .route("/admin/audit/{id}", get(raisfast::handlers::audit::get))
        .route(
            "/admin/webhooks",
            get(raisfast::webhook::handler::list).post(raisfast::webhook::handler::create),
        )
        .route(
            "/admin/webhooks/{id}",
            get(raisfast::webhook::handler::get)
                .put(raisfast::webhook::handler::update)
                .delete(raisfast::webhook::handler::delete),
        )
        .route(
            "/admin/workflows",
            get(raisfast::workflow::handler::list).post(raisfast::workflow::handler::create),
        )
        .route(
            "/admin/workflows/{id}",
            get(raisfast::workflow::handler::get).delete(raisfast::workflow::handler::delete),
        )
        .route(
            "/admin/workflows/{id}/start",
            http_post(raisfast::workflow::handler::start),
        )
        .route(
            "/admin/workflows/instances",
            get(raisfast::workflow::handler::list_instances),
        )
        .route(
            "/admin/workflows/instances/{id}",
            get(raisfast::workflow::handler::get_instance),
        )
        .route(
            "/admin/workflows/instances/{id}/execute",
            http_post(raisfast::workflow::handler::execute_step),
        )
        .route(
            "/admin/workflows/instances/{id}/cancel",
            http_post(raisfast::workflow::handler::cancel_instance),
        )
        .route(
            "/admin/workflows/instances/{id}/logs",
            get(raisfast::workflow::handler::get_step_logs),
        )
        .route("/pages", get(h_page::list).post(h_page::create))
        .route(
            "/pages/{slug}",
            get(h_page::get_by_slug)
                .put(h_page::update)
                .delete(h_page::delete),
        )
        .route("/admin/pages", get(h_page::admin_list))
        .route(
            "/admin/pages/{id}",
            get(h_page::admin_get)
                .put(h_page::update)
                .delete(h_page::delete),
        )
        .route("/admin/pages/{id}/status", put(h_page::update_status))
        .route(
            "/admin/reusable-blocks",
            get(h_block::list_reusable).post(h_block::create_reusable),
        )
        .route(
            "/admin/reusable-blocks/{id}",
            get(h_block::get_reusable)
                .put(h_block::update_reusable)
                .delete(h_block::delete_reusable),
        )
        .route("/products", get(h_product::list_active))
        .route("/products/{slug}", get(h_product::get_product))
        .route(
            "/admin/products",
            get(h_product::admin_list).post(h_product::admin_create),
        )
        .route("/admin/products/batch", http_post(h_product::admin_batch))
        .route(
            "/admin/products/{id}",
            get(h_product::admin_get)
                .put(h_product::admin_update)
                .delete(h_product::admin_delete),
        )
        .route(
            "/product-categories",
            get(h_product_category::list).post(h_product_category::create),
        )
        .route(
            "/product-categories/{id}",
            get(h_product_category::get)
                .put(h_product_category::update)
                .delete(h_product_category::delete),
        )
        .route(
            "/admin/product-categories",
            get(h_product_category::admin_list).post(h_product_category::admin_create),
        )
        .route(
            "/admin/product-categories/{id}",
            put(h_product_category::admin_update).delete(h_product_category::admin_delete),
        )
        .route(
            "/admin/product-categories/batch",
            http_post(h_product_category::admin_batch),
        )
        .route(
            "/orders",
            get(h_order::list_orders).post(h_order::create_order),
        )
        .route(
            "/orders/{id}",
            get(h_order::get_order).put(h_order::cancel_order_handler),
        )
        .route("/orders/{id}/confirm", http_post(h_order::confirm_receipt))
        .route("/admin/orders", get(h_order::admin_list))
        .route("/admin/orders/{id}", get(h_order::admin_get))
        .route("/admin/orders/{id}/pay", http_post(h_order::admin_pay))
        .route("/admin/orders/{id}/ship", http_post(h_order::admin_ship))
        .route(
            "/admin/orders/{id}/cancel",
            http_post(h_order::admin_cancel),
        )
        .route(
            "/admin/orders/{id}/refund",
            http_post(h_order::admin_refund),
        )
        .route(
            "/admin/orders/{id}/remark",
            put(h_order::admin_update_remark),
        )
        .route("/admin/orders/stats", get(h_order::admin_stats))
        .route("/wallets", get(h_wallet::list_wallets))
        .route("/wallets/{currency}", get(h_wallet::get_wallet))
        .route(
            "/wallets/transactions",
            get(h_wallet::list_all_transactions),
        )
        .route(
            "/wallets/{currency}/transactions",
            get(h_wallet::list_transactions),
        )
        .route("/admin/wallets", get(h_wallet::list_all_wallets))
        .route(
            "/admin/wallets/transactions",
            get(h_wallet::list_all_transactions_admin),
        )
        .route("/admin/wallets/credit", http_post(h_wallet::admin_credit))
        .route("/admin/wallets/debit", http_post(h_wallet::admin_debit))
        .route(
            "/admin/wallets/{user_id}/transactions",
            get(h_wallet::list_user_all_transactions),
        )
        .route(
            "/admin/wallets/{user_id}/{currency}/transactions",
            get(h_wallet::list_user_transactions),
        )
        .route(
            "/admin/wallets/{tx_id}/reversal",
            http_post(h_wallet::admin_reversal),
        )
        .route(
            "/payment/channels/available",
            get(h_payment::list_available_channels_handler),
        )
        .route(
            "/payment/orders",
            get(h_payment::list_user_orders).post(h_payment::create_payment_order_handler),
        )
        .route(
            "/payment/orders/{id}",
            get(h_payment::get_payment_order_handler),
        )
        .route(
            "/payment/orders/{id}/cancel",
            http_post(h_payment::cancel_payment_order_handler),
        )
        .route(
            "/payment/orders/{id}/transactions",
            get(h_payment::list_order_transactions),
        )
        .route(
            "/payment/orders/{id}/refunds",
            get(h_payment::list_order_refunds),
        )
        .route(
            "/payment/callback/{channel_id}",
            http_post(h_payment::handle_callback).layer(from_fn(payment_callback_rate_limit)),
        )
        .route(
            "/admin/payment/channels",
            get(h_payment::admin_list_channels).post(h_payment::admin_create_channel),
        )
        .route(
            "/admin/payment/channels/{id}",
            get(h_payment::admin_get_channel)
                .put(h_payment::admin_update_channel)
                .delete(h_payment::admin_delete_channel),
        )
        .route("/admin/payment/orders", get(h_payment::admin_list_orders))
        .route(
            "/admin/payment/orders/{id}",
            get(h_payment::admin_get_order),
        )
        .route(
            "/admin/payment/orders/{id}/refund",
            http_post(h_payment::admin_refund_order),
        )
        .route(
            "/admin/payment/transactions",
            get(h_payment::admin_list_transactions),
        )
        .route("/admin/payment/refunds", get(h_payment::admin_list_refunds))
        // ── Cart ──
        .route("/cart", http_post(h_cart::add_to_cart))
        .route("/cart", get(h_cart::list_cart))
        .route("/cart/{id}", put(h_cart::update_cart_item))
        .route("/cart/{id}", delete(h_cart::remove_from_cart))
        .route("/cart", delete(h_cart::clear_cart))
        .route("/cart/checkout", http_post(h_cart::checkout))
        // ── Product Variants ──
        .route(
            "/products/{product_id}/variants",
            get(h_product_variant::list_by_product),
        )
        .route(
            "/admin/product-variants",
            http_post(h_product_variant::admin_create),
        )
        .route(
            "/admin/product-variants/{id}",
            put(h_product_variant::admin_update),
        )
        .route(
            "/admin/product-variants/{id}",
            delete(h_product_variant::admin_delete),
        )
        // ── User Addresses ──
        .route("/user/addresses", get(h_user_address::list_addresses))
        .route("/user/addresses", http_post(h_user_address::create_address))
        .route("/user/addresses/{id}", put(h_user_address::update_address))
        .route(
            "/user/addresses/{id}",
            delete(h_user_address::delete_address),
        )
        .layer(from_fn(global_rate_limit))
        .layer(axum::Extension(RateLimiterSet::new_default()));

    let app = axum::Router::new()
        .route("/health", get(h_health::health))
        .route("/feed.xml", get(h_rss::feed))
        .nest("/api/v1", api_v1)
        .layer(from_fn(locale_middleware))
        .with_state(state.clone());

    (app, state)
}

pub(crate) async fn send(app: &mut axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

pub(crate) async fn send_raw(app: &mut axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

pub(crate) fn post_json(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

pub(crate) fn post_json_auth(path: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

pub(crate) fn put_json_auth(path: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

pub(crate) fn get_req(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn get_auth(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn delete_auth(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn make_token(
    _user_id: &str,
    iid: i64,
    role: raisfast::models::user::UserRole,
) -> String {
    raisfast::services::auth::generate_access_token_for_test(
        raisfast::types::snowflake_id::SnowflakeId(iid),
        role,
    )
}

pub(crate) async fn register_and_login(
    app: &mut axum::Router,
    email: &str,
    username: &str,
    password: &str,
) -> (String, String) {
    let (status, body) = send(
        app,
        post_json(
            "/api/v1/auth/register",
            json!({"email": email, "username": username, "password": password}),
        ),
    )
    .await;
    assert!(status.is_success(), "register failed: {status} {body:?}");

    let (status, body) = send(
        app,
        post_json(
            "/api/v1/auth/login",
            json!({"email": email, "password": password}),
        ),
    )
    .await;
    assert!(status.is_success(), "login failed: {status} {body:?}");
    let d = &body["data"];
    (
        d["access_token"].as_str().unwrap().to_string(),
        d["refresh_token"].as_str().unwrap().to_string(),
    )
}

pub(crate) async fn create_admin(pool: &raisfast::db::Pool) -> (i64, String) {
    let hash = raisfast::services::auth::hash_password("AdminPass123!").unwrap();
    let sql = "INSERT INTO users (username, role, status, registered_via) VALUES ('testadmin', 'admin', 'active', 'email') RETURNING id";
    let int_id: i64 = sqlx::query_scalar(sql).fetch_one(pool).await.unwrap();
    let cred_data = serde_json::json!({"password_hash": hash}).to_string();
    let cred_id = raisfast::utils::id::new_id();
    let cred_now = raisfast::utils::tz::now_utc();
    let cred_sql = format!(
        "INSERT INTO user_credentials (id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, 'email', {}, {}, 1, {}, {})",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2),
        raisfast::db::Driver::ph(3),
        raisfast::db::Driver::ph(4),
        raisfast::db::Driver::ph(5),
        raisfast::db::Driver::ph(6)
    );
    sqlx::query(&cred_sql)
        .bind(cred_id)
        .bind(int_id)
        .bind("admin@test.com")
        .bind(&cred_data)
        .bind(cred_now)
        .bind(cred_now)
        .execute(pool)
        .await
        .unwrap();
    (int_id, int_id.to_string())
}

pub(crate) async fn create_author(pool: &raisfast::db::Pool) -> (i64, String) {
    let hash = raisfast::services::auth::hash_password("AuthorPass123!").unwrap();
    let sql = "INSERT INTO users (username, role, status, registered_via) VALUES ('testauthor', 'author', 'active', 'email') RETURNING id";
    let int_id: i64 = sqlx::query_scalar(sql).fetch_one(pool).await.unwrap();
    let cred_data = serde_json::json!({"password_hash": hash}).to_string();
    let cred_id = raisfast::utils::id::new_id();
    let cred_now = raisfast::utils::tz::now_utc();
    let cred_sql = format!(
        "INSERT INTO user_credentials (id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, 'email', {}, {}, 1, {}, {})",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2),
        raisfast::db::Driver::ph(3),
        raisfast::db::Driver::ph(4),
        raisfast::db::Driver::ph(5),
        raisfast::db::Driver::ph(6)
    );
    sqlx::query(&cred_sql)
        .bind(cred_id)
        .bind(int_id)
        .bind("author@test.com")
        .bind(&cred_data)
        .bind(cred_now)
        .bind(cred_now)
        .execute(pool)
        .await
        .unwrap();
    (int_id, int_id.to_string())
}

pub(crate) async fn create_published_post(app: &mut axum::Router, token: &str) -> String {
    let (_, body) = send(
        app,
        post_json_auth(
            "/api/v1/posts",
            json!({"title": "Test Post", "content": "content", "status": "published"}),
            token,
        ),
    )
    .await;
    body["data"]["slug"].as_str().unwrap().to_string()
}

#[path = "api/api_token.rs"]
mod api_token;
#[path = "api/audit.rs"]
mod audit;
#[path = "api/auth.rs"]
mod auth;
#[path = "api/cart.rs"]
mod cart;
#[path = "api/category.rs"]
mod category;
#[path = "api/comment.rs"]
mod comment;
#[path = "api/cron.rs"]
mod cron;
#[path = "api/health.rs"]
mod health;
#[path = "api/media.rs"]
mod media;
#[path = "api/options.rs"]
mod options;
#[path = "api/order.rs"]
mod order;
#[path = "api/page.rs"]
mod page;
#[path = "api/payment.rs"]
mod payment;
#[path = "api/plugin.rs"]
mod plugin;
#[path = "api/post.rs"]
mod post;
#[path = "api/product.rs"]
mod product;
#[path = "api/product_category.rs"]
mod product_category;
#[path = "api/product_variant.rs"]
mod product_variant;
#[path = "api/rbac.rs"]
mod rbac;
#[path = "api/reusable_block.rs"]
mod reusable_block;
#[path = "api/rss.rs"]
mod rss;
#[path = "api/sse.rs"]
mod sse;
#[path = "api/stats.rs"]
mod stats;
#[path = "api/tag.rs"]
mod tag;
#[path = "api/tenant_admin.rs"]
mod tenant_admin;
#[path = "api/tenant_e2e.rs"]
mod tenant_e2e;
#[path = "api/user.rs"]
mod user;
#[path = "api/user_address.rs"]
mod user_address;
#[path = "api/wallet.rs"]
mod wallet;
#[path = "api/webhook.rs"]
mod webhook;
#[path = "api/workflow.rs"]
mod workflow;

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
use axum::extract::Query;
use axum::http::{Request, StatusCode, header};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{delete, get, post, post as http_post, put};
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
    // Never write test content types into the repo's `extensions/content_types`
    // dir; use a temp dir so tests don't pollute real schema files.
    cfg.content_type_dir = std::env::temp_dir()
        .join("raisfast-test-content-types")
        .to_string_lossy()
        .into();
    cfg.base_url = "http://localhost:9000".into();
    let mut key_bytes = [0u8; 32];
    getrandom::fill(&mut key_bytes).unwrap();
    cfg.app_key = Some(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        key_bytes,
    ));
    cfg
}

pub(crate) async fn test_pool() -> raisfast::db::Pool {
    raisfast::test_pool!()
}

pub(crate) async fn test_pool_with_tenants() -> raisfast::db::Pool {
    raisfast::test_pool!()
}

pub(crate) async fn test_app() -> (axum::Router, AppState) {
    build_test_app(test_pool().await).await
}

pub(crate) async fn test_app_with_tenants() -> (axum::Router, AppState) {
    build_test_app(test_pool_with_tenants().await).await
}

async fn build_test_app(pool: raisfast::db::Pool) -> (axum::Router, AppState) {
    let config = Arc::new(test_config());
    let emitter =
        raisfast::event::EventEmitter::eventbus_only(raisfast::eventbus::EventBus::new(256));
    let content_registry = Arc::new(raisfast::content_type::ContentTypeRegistry::new());
    let state = AppState {
        pool: pool.clone(),
        config: config.clone(),
        jwt_decoding_key: jsonwebtoken::DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        plugins: PluginManager::new(config.clone()).await,
        eventbus: raisfast::eventbus::EventBus::new(256),
        post_service: {
            Arc::new(raisfast::services::post::PostServiceImpl::new(
                Arc::new(pool.clone()),
                emitter.clone(),
                Arc::new(NoopSearchEngine),
            ))
        },
        page_service: Arc::new(raisfast::services::page::PageServiceImpl::new(
            emitter.clone(),
            Arc::new(pool.clone()),
        )),
        category_service: Arc::new(raisfast::services::category::CategoryServiceImpl::new(
            emitter.clone(),
            Arc::new(pool.clone()),
        )),
        tag_service: Arc::new(raisfast::services::tag::TagServiceImpl::new(
            emitter.clone(),
            Arc::new(pool.clone()),
        )),
        comment_service: Arc::new(raisfast::services::comment::CommentServiceImpl::new(
            Arc::new(pool.clone()),
            emitter.clone(),
        )),
        wallet_service: Arc::new(raisfast::services::wallet::WalletServiceImpl::new(
            emitter.clone(),
            Arc::new(pool.clone()),
        )),
        product_category_service: Arc::new(
            raisfast::services::product_category::ProductCategoryServiceImpl::new(
                emitter.clone(),
                Arc::new(pool.clone()),
            ),
        ),
        product_service: Arc::new(raisfast::services::product::ProductServiceImpl::new(
            emitter.clone(),
            Arc::new(pool.clone()),
            Arc::new(
                raisfast::services::options::OptionsService::new(Arc::new(pool.clone()), false)
                    .await,
            ),
        )),
        order_service: Arc::new(raisfast::services::order::OrderServiceImpl::new(
            emitter.clone(),
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
            emitter.clone(),
            Arc::new(pool.clone()),
        )),
        user_service: Arc::new(raisfast::services::user::UserServiceImpl::new(Arc::new(
            pool.clone(),
        ))),
        search: Arc::new(NoopSearchEngine),
        content_type_registry: content_registry.clone(),
        emitter: emitter.clone(),
        protocol_registry: Arc::new({
            let mut reg = raisfast::protocols::ProtocolRegistry::new();
            reg.register(raisfast::protocols::ownable::OwnableProtocol);
            reg.register(raisfast::protocols::timestampable::TimestampableProtocol);
            reg
        }),
        options: Arc::new(
            raisfast::services::options::OptionsService::new(Arc::new(pool.clone()), false).await,
        ),
        rbac: Arc::new(raisfast::services::rbac::RbacService::new(
            Arc::new(pool.clone()),
            Arc::new(raisfast::cache::MemoryCache::new()),
        )),
        tenant: Arc::new(raisfast::services::tenant::TenantService::new(Arc::new(
            pool.clone(),
        ))),
        audit: Arc::new(raisfast::services::audit::AuditService::new(pool.clone())),
        webhook: Arc::new(raisfast::webhook::WebhookService::new(pool.clone())),
        integration: Some(Arc::new(
            raisfast::integration::IntegrationPlane::init(
                pool.clone(),
                config.integration.clone(),
                config.storage_root_dir.clone(),
                content_registry.clone(),
                emitter.clone(),
            )
            .await
            .expect("integration plane init"),
        )),
        workflow: Arc::new(raisfast::workflow::WorkflowService::new(pool.clone())),
        storage: raisfast::storage::create_storage(&config).expect("failed to create storage"),
        cache: Arc::new(raisfast::cache::MemoryCache::new()),
        cms_cache: Arc::new(dashmap::DashMap::new()),
        oauth_registry: Arc::new(raisfast::oauth::OAuthProviderRegistry::default()),
        email_sender: raisfast::notifier::build_email_sender(&config),
        sms_sender: raisfast::notifier::build_sms_sender(&config),
        route_registry: Arc::new(Vec::new()),
        route_perms: Arc::new(
            raisfast::middleware::permission_guard::RoutePermissionMap::from_routes(
                &test_route_permissions(),
            ),
        ),
        services: raisfast::app::ServiceRegistry::new(),
        handler_registry: Arc::new(raisfast::worker::JobHandlerRegistry::new()),
    };
    let max_upload = state.config.max_upload_size;

    let mut ct_route_registry = raisfast::server::RouteRegistry::default();

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
        .merge(raisfast::content_type::handler::routes(
            &mut ct_route_registry,
            &config,
        ))
        .route(
            "/ingress/{channel_key}",
            get(raisfast::integration::routes::challenge)
                .post(raisfast::integration::routes::push),
        )
        .route(
            "/admin/integration/channels",
            get(raisfast::integration::admin::list_channels)
                .post(raisfast::integration::admin::create_channel),
        )
        .route(
            "/admin/integration/channels/{id}",
            get(raisfast::integration::admin::get_channel)
                .put(raisfast::integration::admin::update_channel)
                .delete(raisfast::integration::admin::delete_channel),
        )
        .route(
            "/admin/integration/channels/{id}/test-mapping",
            post(raisfast::integration::admin::test_mapping),
        )
        .route(
            "/admin/integration/receipts",
            get(raisfast::integration::admin::list_receipts),
        )
        .route(
            "/admin/integration/receipts/{id}",
            get(raisfast::integration::admin::get_receipt),
        )
        .route(
            "/admin/integration/receipts/{id}/trace",
            get(raisfast::integration::admin::get_trace),
        )
        .layer(from_fn_with_state(
            state.clone(),
            raisfast::middleware::permission_guard::permission_guard,
        ))
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
        vec![role],
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

pub(crate) fn uniq(prefix: &str) -> String {
    format!("{}_{}", prefix, raisfast::utils::id::new_id())
}

/// Generate a unique email from a prefix. `uniq_email("login")` → `"login_123@test.com"`
pub(crate) fn uniq_email(prefix: &str) -> String {
    format!("{}_{}@test.com", prefix, raisfast::utils::id::new_id())
}

pub(crate) async fn create_admin(pool: &raisfast::db::Pool) -> (i64, String) {
    let hash = raisfast::services::auth::hash_password("AdminPass123!").unwrap();
    let uid = raisfast::utils::id::new_id();
    let uname = format!("testadmin_{}", raisfast::utils::id::new_id());
    let email = format!("admin_{}@test.com", raisfast::utils::id::new_id());
    let sql = format!(
        "INSERT INTO users (id, username, status, registered_via) VALUES ({}, {}, 'active', 'email')",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2)
    );
    sqlx::query(raisfast::db::safe_sql(&sql))
        .bind(uid)
        .bind(&uname)
        .execute(pool)
        .await
        .unwrap();
    let int_id = uid;
    let admin_rid = raisfast::models::rbac::find_role_id_by_name(pool, "admin")
        .await
        .unwrap()
        .unwrap();
    raisfast::models::user_role::assign_role(
        pool,
        raisfast::types::snowflake_id::SnowflakeId(int_id),
        raisfast::types::snowflake_id::SnowflakeId(admin_rid),
        "default",
    )
    .await
    .unwrap();
    let cred_data = serde_json::json!({"password_hash": hash});
    let cred_id = raisfast::utils::id::new_id();
    let cred_now = raisfast::utils::tz::now_utc();
    let cred_sql = format!(
        "INSERT INTO user_credentials (id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, 'email', {}, {}, true, {}, {})",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2),
        raisfast::db::Driver::ph(3),
        raisfast::db::Driver::ph(4),
        raisfast::db::Driver::ph(5),
        raisfast::db::Driver::ph(6)
    );
    sqlx::query(raisfast::db::safe_sql(&cred_sql))
        .bind(cred_id)
        .bind(int_id)
        .bind(&email)
        .bind(&cred_data)
        .bind(cred_now)
        .bind(cred_now)
        .execute(pool)
        .await
        .unwrap();
    ADMIN_EMAIL.with(|c| *c.borrow_mut() = email.clone());
    (int_id, int_id.to_string())
}

thread_local! {
    static ADMIN_EMAIL: std::cell::RefCell<String> = std::cell::RefCell::new("admin@test.com".into());
}

pub(crate) async fn create_author(pool: &raisfast::db::Pool) -> (i64, String) {
    let hash = raisfast::services::auth::hash_password("AuthorPass123!").unwrap();
    let uid = raisfast::utils::id::new_id();
    let uname = format!("testauthor_{}", raisfast::utils::id::new_id());
    let email = format!("author_{}@test.com", raisfast::utils::id::new_id());
    let sql = format!(
        "INSERT INTO users (id, username, status, registered_via) VALUES ({}, {}, 'active', 'email')",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2)
    );
    sqlx::query(raisfast::db::safe_sql(&sql))
        .bind(uid)
        .bind(&uname)
        .execute(pool)
        .await
        .unwrap();
    let int_id = uid;
    let author_rid = raisfast::models::rbac::find_role_id_by_name(pool, "author")
        .await
        .unwrap()
        .unwrap();
    raisfast::models::user_role::assign_role(
        pool,
        raisfast::types::snowflake_id::SnowflakeId(int_id),
        raisfast::types::snowflake_id::SnowflakeId(author_rid),
        "default",
    )
    .await
    .unwrap();
    let cred_data = serde_json::json!({"password_hash": hash});
    let cred_id = raisfast::utils::id::new_id();
    let cred_now = raisfast::utils::tz::now_utc();
    let cred_sql = format!(
        "INSERT INTO user_credentials (id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, 'email', {}, {}, true, {}, {})",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2),
        raisfast::db::Driver::ph(3),
        raisfast::db::Driver::ph(4),
        raisfast::db::Driver::ph(5),
        raisfast::db::Driver::ph(6)
    );
    sqlx::query(raisfast::db::safe_sql(&cred_sql))
        .bind(cred_id)
        .bind(int_id)
        .bind(&email)
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
#[path = "api/permissions.rs"]
mod permissions;
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

/// Build route permission declarations matching the test app's routes.
///
/// Mirrors the permissions declared in the production handler `routes()` functions
/// so the `permission_guard` middleware enforces the same access control in tests.
fn test_route_permissions() -> Vec<raisfast::server::RouteInfo> {
    use raisfast::server::RouteInfo;

    fn r(method: &str, path: &str, perm: &str) -> RouteInfo {
        RouteInfo {
            method: method.to_string(),
            path: path.to_string(),
            source: "test".to_string(),
            source_name: "test".to_string(),
            permission: Some(perm.to_string()),
        }
    }

    vec![
        // ── Auth (public) ──
        r("POST", "/api/v1/auth/register", "public"),
        r("POST", "/api/v1/auth/login", "public"),
        r("POST", "/api/v1/auth/refresh", "public"),
        r("POST", "/api/v1/auth/logout", "authed"),
        // ── Tokens ──
        r("GET", "/api/v1/tokens", "authed"),
        r("POST", "/api/v1/tokens", "authed"),
        r("DELETE", "/api/v1/tokens/{id}", "authed"),
        // ── Users ──
        r("GET", "/api/v1/users/me", "authed"),
        r("PUT", "/api/v1/users/me", "authed"),
        r("PUT", "/api/v1/users/me/password", "authed"),
        r("GET", "/api/v1/users/{id}", "public"),
        r("PUT", "/api/v1/users/{id}/role", "admin"),
        r("GET", "/api/v1/users", "admin"),
        // ── Categories ──
        r("GET", "/api/v1/categories", "public"),
        r("POST", "/api/v1/categories", "categories:create"),
        r("PUT", "/api/v1/categories/{id}", "categories:update"),
        r("DELETE", "/api/v1/categories/{id}", "categories:delete"),
        // ── Tags ──
        r("GET", "/api/v1/tags", "public"),
        r("POST", "/api/v1/tags", "tags:create"),
        r("DELETE", "/api/v1/tags/{id}", "tags:delete"),
        // ── Posts ──
        r("GET", "/api/v1/posts", "public"),
        r("POST", "/api/v1/posts", "posts:create"),
        r("GET", "/api/v1/posts/{slug}", "public"),
        r("PUT", "/api/v1/posts/{slug}", "posts:update"),
        r("DELETE", "/api/v1/posts/{slug}", "posts:delete"),
        // ── Comments ──
        r("GET", "/api/v1/posts/{slug}/comments", "public"),
        r("POST", "/api/v1/posts/{slug}/comments", "public"),
        r(
            "POST",
            "/api/v1/posts/{slug}/comments/authed",
            "comments:create",
        ),
        r("DELETE", "/api/v1/comments/{id}", "comments:delete"),
        r("PUT", "/api/v1/comments/{id}/status", "admin"),
        // ── Media ──
        r("POST", "/api/v1/media/upload", "media:create"),
        r("GET", "/api/v1/media", "media:read"),
        r("DELETE", "/api/v1/media/{id}", "media:delete"),
        // ── Pages ──
        r("GET", "/api/v1/pages", "public"),
        r("POST", "/api/v1/pages", "pages:create"),
        r("GET", "/api/v1/pages/{slug}", "public"),
        r("PUT", "/api/v1/pages/{slug}", "pages:update"),
        r("DELETE", "/api/v1/pages/{slug}", "pages:delete"),
        r("GET", "/api/v1/admin/pages", "pages:read"),
        r("GET", "/api/v1/admin/pages/{id}", "pages:read"),
        r("PUT", "/api/v1/admin/pages/{id}", "pages:update"),
        r("DELETE", "/api/v1/admin/pages/{id}", "pages:delete"),
        r("PUT", "/api/v1/admin/pages/{id}/status", "pages:update"),
        // ── Reusable Blocks ──
        r(
            "GET",
            "/api/v1/admin/reusable-blocks",
            "reusable_blocks:read",
        ),
        r(
            "POST",
            "/api/v1/admin/reusable-blocks",
            "reusable_blocks:create",
        ),
        r(
            "GET",
            "/api/v1/admin/reusable-blocks/{id}",
            "reusable_blocks:read",
        ),
        r(
            "PUT",
            "/api/v1/admin/reusable-blocks/{id}",
            "reusable_blocks:update",
        ),
        r(
            "DELETE",
            "/api/v1/admin/reusable-blocks/{id}",
            "reusable_blocks:delete",
        ),
        // ── Products ──
        r("GET", "/api/v1/products", "public"),
        r("GET", "/api/v1/products/{slug}", "public"),
        r("GET", "/api/v1/admin/products", "admin"),
        r("POST", "/api/v1/admin/products", "admin"),
        r("POST", "/api/v1/admin/products/batch", "admin"),
        r("GET", "/api/v1/admin/products/{id}", "admin"),
        r("PUT", "/api/v1/admin/products/{id}", "admin"),
        r("DELETE", "/api/v1/admin/products/{id}", "admin"),
        // ── Product Categories ──
        r("GET", "/api/v1/product-categories", "public"),
        r(
            "POST",
            "/api/v1/product-categories",
            "product_categories:create",
        ),
        r("GET", "/api/v1/product-categories/{id}", "public"),
        r(
            "PUT",
            "/api/v1/product-categories/{id}",
            "product_categories:update",
        ),
        r(
            "DELETE",
            "/api/v1/product-categories/{id}",
            "product_categories:delete",
        ),
        // ── Orders ──
        r("GET", "/api/v1/orders", "orders:read"),
        r("POST", "/api/v1/orders", "orders:create"),
        r("GET", "/api/v1/orders/{id}", "orders:read"),
        r("PUT", "/api/v1/orders/{id}/cancel", "orders:update"),
        r("PUT", "/api/v1/admin/orders/{id}/remark", "admin"),
        r("GET", "/api/v1/admin/orders/stats", "admin"),
        // ── Wallets ──
        r("GET", "/api/v1/wallets", "wallet:read"),
        r("GET", "/api/v1/wallets/{currency}", "wallet:read"),
        r("GET", "/api/v1/wallets/transactions", "wallet:read"),
        r(
            "GET",
            "/api/v1/wallets/{currency}/transactions",
            "wallet:read",
        ),
        r("GET", "/api/v1/admin/wallets", "admin"),
        r("GET", "/api/v1/admin/wallets/transactions", "admin"),
        r("POST", "/api/v1/admin/wallets/credit", "admin"),
        r("POST", "/api/v1/admin/wallets/debit", "admin"),
        r(
            "GET",
            "/api/v1/admin/wallets/{user_id}/transactions",
            "admin",
        ),
        r(
            "GET",
            "/api/v1/admin/wallets/{user_id}/{currency}/transactions",
            "admin",
        ),
        r("POST", "/api/v1/admin/wallets/{tx_id}/reversal", "admin"),
        // ── Payment ──
        r("GET", "/api/v1/payment/channels/available", "public"),
        r("GET", "/api/v1/payment/orders", "payment:read"),
        r("POST", "/api/v1/payment/orders", "payment:create"),
        r("GET", "/api/v1/payment/orders/{id}", "payment:read"),
        r(
            "POST",
            "/api/v1/payment/orders/{id}/cancel",
            "payment:update",
        ),
        r(
            "GET",
            "/api/v1/payment/orders/{id}/transactions",
            "payment:read",
        ),
        r("GET", "/api/v1/payment/orders/{id}/refunds", "payment:read"),
        r("POST", "/api/v1/payment/callback/{channel_id}", "public"),
        // ── Cart ──
        r("POST", "/api/v1/cart", "cart_items:create"),
        r("GET", "/api/v1/cart", "cart_items:read"),
        r("PUT", "/api/v1/cart/{id}", "cart_items:update"),
        r("DELETE", "/api/v1/cart/{id}", "cart_items:delete"),
        r("DELETE", "/api/v1/cart", "cart_items:delete"),
        r("POST", "/api/v1/cart/checkout", "cart_items:create"),
        // ── Product Variants ──
        r("GET", "/api/v1/products/{product_id}/variants", "public"),
        r("POST", "/api/v1/admin/product-variants", "admin"),
        r("PUT", "/api/v1/admin/product-variants/{id}", "admin"),
        r("DELETE", "/api/v1/admin/product-variants/{id}", "admin"),
        // ── User Addresses ──
        r("GET", "/api/v1/user/addresses", "user_addresses:read"),
        r("POST", "/api/v1/user/addresses", "user_addresses:create"),
        r(
            "PUT",
            "/api/v1/user/addresses/{id}",
            "user_addresses:update",
        ),
        r(
            "DELETE",
            "/api/v1/user/addresses/{id}",
            "user_addresses:delete",
        ),
        // ── Admin (heuristic covers these, but explicit for clarity) ──
        // All /admin/ routes without explicit permission above are caught by heuristic
    ]
}

// ── Content type: blob + media_set CRUD via HTTP ────────────────────

#[tokio::test]
async fn content_type_blob_media_set_crud_api() {
    let (mut app, state) = test_app().await;
    let (admin_pk, _) = create_admin(&state.pool).await;

    let (status, body) = send(
        &mut app,
        post_json(
            "/api/v1/auth/login",
            json!({ "email": ADMIN_EMAIL.with(|c| c.borrow().clone()), "password": "AdminPass123!" }),
        ),
    )
    .await;
    assert!(status.is_success(), "admin login failed: {status} {body:?}");
    let token = body["data"]["access_token"].as_str().unwrap().to_string();

    let schema = json!({
        "name": "Docs",
        "singular": "doc",
        "plural": "docs",
        "table": "docs",
        "implements": ["ownable", "timestampable"],
        "fields": [
            { "name": "title", "label": "Title", "field_type": "text", "required": true },
            { "name": "payload", "label": "Payload", "field_type": "blob" },
            { "name": "gallery", "label": "Gallery", "field_type": "media_set", "media_config": { "accept": [], "max_count": 5 } }
        ]
    });
    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/content-types", schema, &token),
    )
    .await;
    assert!(
        status.is_success(),
        "create schema failed: {status} {body:?}"
    );

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"hello blob");

    // Seed real media records for the gallery relation
    let mut gallery_ids = Vec::new();
    for name in &["g1.png", "g2.png"] {
        let cmd = raisfast::commands::CreateMediaCmd {
            user_id: raisfast::types::snowflake_id::SnowflakeId(admin_pk),
            filename: name.to_string(),
            filepath: format!("/uploads/{name}"),
            mimetype: "image/png".to_string(),
            size: 42,
            width: None,
            height: None,
        };
        let media = raisfast::models::media::create(&state.pool, &cmd, None)
            .await
            .unwrap();
        gallery_ids.push(media.id.to_string());
    }

    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/cms/docs",
            json!({
                "title": "Doc One",
                "payload": { "data": b64, "filename": "a.json", "mimetype": "application/json" },
                "gallery": gallery_ids
            }),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "create record failed: {status} {body:?}"
    );
    let id = body["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["payload"]["filename"], "payload.txt");
    assert_eq!(body["data"]["payload"]["data"], json!(b64));
    assert_eq!(body["data"]["gallery"].as_array().unwrap().len(), 2);
    assert!(body["data"].get("payload_meta").is_none());
    assert!(body["data"].get("gallery_meta").is_none());

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/cms/docs/{id}"), &token),
    )
    .await;
    assert!(status.is_success(), "get failed: {status} {body:?}");
    assert_eq!(body["data"]["payload"]["filename"], "payload.txt");
    assert_eq!(body["data"]["gallery"].as_array().unwrap().len(), 2);

    let b64b = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"updated");
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/cms/docs/{id}"),
            json!({
                "payload": { "data": b64b, "filename": "b.bin", "mimetype": "application/octet-stream" },
                "gallery": [gallery_ids[0].clone()]
            }),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "update failed: {status} {body:?}");
    assert_eq!(body["data"]["payload"]["data"], json!(b64b));
    assert_eq!(body["data"]["payload"]["filename"], "payload.txt");
    assert_eq!(body["data"]["gallery"].as_array().unwrap().len(), 1);

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/cms/docs/{id}"), &token),
    )
    .await;
    assert!(status.is_success(), "delete failed: {status}");

    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/cms/docs/{id}"), &token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── Content type: blob + media_set via public (non-admin) API ─────────

#[tokio::test]
async fn cms_public_api_blob_media_set_crud_non_admin() {
    let (mut app, state) = test_app().await;

    let toml = r#"
[content_type]
name = "ApiDocs"
singular = "api_doc"
plural = "api_docs"
table = "api_docs"
implements = ["timestampable"]

[api]
[api.list]
access = "authed"
[api.get]
access = "authed"
[api.create]
access = "authed"
[api.update]
access = "authed"
[api.delete]
access = "authed"

[fields.title]
type = "text"
required = true

[fields.payload]
type = "blob"

[fields.cover]
type = "media"

[fields.gallery]
type = "mediaset"
"#;
    let mut schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    schema.cache_protocol_columns(&state.protocol_registry);
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry)
        .await
        .unwrap();
    state
        .content_type_registry
        .register(
            schema.clone(),
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    let api_email = uniq_email("api");
    let api_user = uniq("apiuser");
    let (status, body) = send(
        &mut app,
        post_json(
            "/api/v1/auth/register",
            json!({ "email": &api_email, "username": &api_user, "password": "ApiPass123!" }),
        ),
    )
    .await;
    assert!(status.is_success(), "register failed: {status} {body:?}");
    let (status, body) = send(
        &mut app,
        post_json(
            "/api/v1/auth/login",
            json!({ "email": &api_email, "password": "ApiPass123!" }),
        ),
    )
    .await;
    assert!(status.is_success(), "login failed: {status} {body:?}");
    let token = body["data"]["access_token"].as_str().unwrap().to_string();

    let user_id: i64 = sqlx::query_scalar(raisfast::db::safe_sql(&format!(
        "SELECT id FROM users WHERE username = {}",
        raisfast::db::Driver::ph(1)
    )))
    .bind(&api_user)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let user_pk = raisfast::types::snowflake_id::SnowflakeId(user_id);
    let mut media_ids = Vec::new();
    for (name, mime) in [
        ("a.png", "image/png"),
        ("b.pdf", "application/pdf"),
        ("c.bin", "application/octet-stream"),
    ] {
        let cmd = raisfast::commands::CreateMediaCmd {
            user_id: user_pk,
            filename: name.to_string(),
            filepath: format!("/uploads/{name}"),
            mimetype: mime.to_string(),
            size: 42,
            width: None,
            height: None,
        };
        let media = raisfast::models::media::create(&state.pool, &cmd, None)
            .await
            .unwrap();
        media_ids.push(media.id.to_string());
    }
    let cover_id = media_ids[0].clone();
    let gallery_ids = vec![media_ids[1].clone(), media_ids[2].clone()];

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"api blob");
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/cms/api_docs",
            json!({
                "title": "Api Doc",
                "payload": { "data": b64, "filename": "api.json", "mimetype": "application/json" },
                "cover": cover_id,
                "gallery": gallery_ids
            }),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "public create failed: {status} {body:?}"
    );
    let id = body["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["payload"]["filename"], "payload.txt");
    // cover is stored as a JSON string value; may be double-quoted depending on serialization
    let cover_raw = &body["data"]["cover"];
    let cover_actual = cover_raw.as_str().unwrap_or("");
    let cover_unquoted = if cover_actual.starts_with('"') && cover_actual.ends_with('"') {
        &cover_actual[1..cover_actual.len() - 1]
    } else {
        cover_actual
    };
    assert_eq!(cover_unquoted, cover_id);
    assert_eq!(body["data"]["gallery"].as_array().unwrap().len(), 2);

    assert!(body["data"].get("payload_meta").is_none());

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/cms/api_docs/{id}"), &token),
    )
    .await;
    assert!(status.is_success(), "public get failed: {status} {body:?}");
    assert_eq!(body["data"]["payload"]["filename"], "payload.txt");
    let cover_actual = body["data"]["cover"].as_str().unwrap_or("");
    let cover_unquoted = if cover_actual.starts_with('"') && cover_actual.ends_with('"') {
        &cover_actual[1..cover_actual.len() - 1]
    } else {
        cover_actual
    };
    assert_eq!(cover_unquoted, cover_id);

    let b64b = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"api updated");
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/cms/api_docs/{id}"),
            json!({
                "payload": { "data": b64b, "filename": "u.bin", "mimetype": "application/octet-stream" },
                "cover": media_ids[2],
                "gallery": [media_ids[0]]
            }),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "public update failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["payload"]["data"], json!(b64b));
    let cover_actual = body["data"]["cover"].as_str().unwrap_or("");
    let cover_unquoted = if cover_actual.starts_with('"') && cover_actual.ends_with('"') {
        &cover_actual[1..cover_actual.len() - 1]
    } else {
        cover_actual
    };
    assert_eq!(cover_unquoted, media_ids[2]);
    assert_eq!(body["data"]["gallery"].as_array().unwrap().len(), 1);

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/cms/api_docs/{id}"), &token),
    )
    .await;
    assert!(status.is_success(), "public delete failed: {status}");
    let (status, _) = send(
        &mut app,
        get_auth(&format!("/api/v1/cms/api_docs/{id}"), &token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── Content type: ?filter= expression query ─────────────────────────

#[tokio::test]
#[ignore = "pre-existing PG issue: shared DB data accumulation"]
async fn cms_list_filter_expression_query_param() {
    let (mut app, state) = test_app().await;
    create_admin(&state.pool).await;

    let (status, body) = send(
        &mut app,
        post_json(
            "/api/v1/auth/login",
            json!({ "email": ADMIN_EMAIL.with(|c| c.borrow().clone()), "password": "AdminPass123!" }),
        ),
    )
    .await;
    assert!(status.is_success(), "admin login failed: {status} {body:?}");
    let token = body["data"]["access_token"].as_str().unwrap().to_string();

    let schema = json!({
        "name": "Gadget",
        "singular": "gadget",
        "plural": "gadgets",
        "table": "gadgets",
        "implements": ["ownable", "timestampable"],
        "api": {
            "list": { "access": "public" },
            "get": { "access": "public" }
        },
        "fields": [
            { "name": "title", "field_type": "text", "required": true },
            { "name": "price", "field_type": "integer", "required": true }
        ]
    });
    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/content-types", schema, &token),
    )
    .await;
    assert!(
        status.is_success(),
        "create schema failed: {status} {body:?}"
    );

    for (title, price) in [("Cheap", 10), ("Mid", 300), ("Mid2", 450), ("Pricey", 900)] {
        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/cms/gadgets",
                json!({ "title": title, "price": price }),
                &token,
            ),
        )
        .await;
        assert!(
            status.is_success(),
            "create {title} failed: {status} {body:?}"
        );
    }

    // filter=price>=100&&price<=500  → only Mid and Mid2
    let (status, body) = send(
        &mut app,
        get_req("/api/v1/cms/gadgets?filter=price%3E%3D100%26%26price%3C%3D500"),
    )
    .await;
    assert!(status.is_success(), "filter list failed: {status} {body:?}");
    assert_eq!(body["data"]["total"], 2);
    let titles: Vec<String> = body["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["title"].as_str().unwrap().to_string())
        .collect();
    assert!(titles.contains(&"Mid".to_string()));
    assert!(titles.contains(&"Mid2".to_string()));

    // filter=title="Mid" → exactly one
    let (status, body) = send(
        &mut app,
        get_req("/api/v1/cms/gadgets?filter=title%3D%22Mid%22"),
    )
    .await;
    assert!(
        status.is_success(),
        "filter title failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["total"], 1);
    assert_eq!(body["data"]["items"][0]["title"], "Mid");

    // malformed filter is ignored (returns all)
    let (status, body) = send(
        &mut app,
        get_req("/api/v1/cms/gadgets?filter=price%3E%3E%3E100"),
    )
    .await;
    assert!(
        status.is_success(),
        "malformed filter failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["total"], 4);

    // combine filter with bracket operator params (AND)
    let (status, body) = send(
        &mut app,
        get_req("/api/v1/cms/gadgets?filter=price%3E%3D100&price%5B%24lt%5D=500"),
    )
    .await;
    assert!(
        status.is_success(),
        "combined filter failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["total"], 2);
}

// ── Content type: full-table export ──────────────────────────────

/// Test app backed by a temp-file SQLite DB.
///
/// The export pipeline runs on a dedicated thread + single-threaded runtime,
/// which exposes SQLite `:memory:`'s per-connection isolation. A real file
/// (as in production) shares the schema across every pool connection.
async fn test_app_export() -> (axum::Router, AppState, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "raisfast-export-{}-{}.db",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let pool = raisfast::db::Pool::connect(&url).await.unwrap();
    sqlx::query(raisfast::db::schema::SCHEMA_SQL)
        .execute(&pool)
        .await
        .unwrap();
    let (app, state) = build_test_app(pool).await;
    (app, state, path)
}

#[tokio::test]
#[ignore = "pre-existing PG issue: shared DB data accumulation"]
async fn cms_export_streams_all_formats() {
    let (mut app, state, db_path) = test_app_export().await;
    create_admin(&state.pool).await;

    let (status, body) = send(
        &mut app,
        post_json(
            "/api/v1/auth/login",
            json!({ "email": ADMIN_EMAIL.with(|c| c.borrow().clone()), "password": "AdminPass123!" }),
        ),
    )
    .await;
    assert!(status.is_success(), "admin login failed: {status} {body:?}");
    let token = body["data"]["access_token"].as_str().unwrap().to_string();

    let schema = json!({
        "name": "Widget",
        "singular": "widget",
        "plural": "widgets",
        "table": "widgets",
        "implements": ["ownable", "timestampable"],
        "fields": [
            { "name": "title", "field_type": "text", "required": true },
            { "name": "price", "field_type": "integer", "required": true }
        ]
    });
    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/content-types", schema, &token),
    )
    .await;
    assert!(
        status.is_success(),
        "create schema failed: {status} {body:?}"
    );

    for (title, price) in [("A", 1), ("B", 2), ("C", 3)] {
        let (status, body) = send(
            &mut app,
            post_json_auth(
                "/api/v1/admin/cms/widgets",
                json!({ "title": title, "price": price }),
                &token,
            ),
        )
        .await;
        assert!(
            status.is_success(),
            "create {title} failed: {status} {body:?}"
        );
    }

    // JSON export → valid array of 3
    let (status, bytes) = send_raw(
        &mut app,
        get_auth("/api/v1/admin/cms/widgets/export?format=json", &token),
    )
    .await;
    assert!(status.is_success(), "json export failed: {status}");
    let parsed: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 3);

    // CSV export → header + rows
    let (status, bytes) = send_raw(
        &mut app,
        get_auth("/api/v1/admin/cms/widgets/export?format=csv", &token),
    )
    .await;
    assert!(status.is_success(), "csv export failed: {status}");
    let csv_text = String::from_utf8(bytes).unwrap();
    assert!(csv_text.contains("title"));
    assert!(csv_text.contains("price"));
    assert!(csv_text.contains("A"));

    // SQL export → INSERT statements
    let (status, bytes) = send_raw(
        &mut app,
        get_auth("/api/v1/admin/cms/widgets/export?format=sql", &token),
    )
    .await;
    assert!(status.is_success(), "sql export failed: {status}");
    let sql_text = String::from_utf8(bytes).unwrap();
    assert!(sql_text.contains("INSERT INTO `widgets`"));

    // XLSX export → zip magic bytes
    let (status, bytes) = send_raw(
        &mut app,
        get_auth("/api/v1/admin/cms/widgets/export?format=xlsx", &token),
    )
    .await;
    assert!(status.is_success(), "xlsx export failed: {status}");
    assert!(bytes.starts_with(b"PK"));

    // unsupported format → 400
    let (status, _) = send_raw(
        &mut app,
        get_auth("/api/v1/admin/cms/widgets/export?format=txt", &token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // empty table → 400
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/content-types",
            json!({
                "name": "Empty",
                "singular": "empty",
                "plural": "empties",
                "table": "empties",
                "implements": ["ownable"],
                "fields": [{ "name": "title", "field_type": "text" }]
            }),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "create empty schema failed: {status} {body:?}"
    );
    let (status, _) = send_raw(
        &mut app,
        get_auth("/api/v1/admin/cms/empties/export?format=json", &token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let _ = std::fs::remove_file(&db_path);
}

// ── Content type: API config update ─────────────────────────────

#[tokio::test]
#[ignore = "pre-existing PG issue: shared DB data accumulation"]
async fn cms_content_type_api_config_update() {
    let (mut app, state, db_path) = test_app_export().await;
    create_admin(&state.pool).await;

    let (status, body) = send(
        &mut app,
        post_json(
            "/api/v1/auth/login",
            json!({ "email": ADMIN_EMAIL.with(|c| c.borrow().clone()), "password": "AdminPass123!" }),
        ),
    )
    .await;
    assert!(status.is_success(), "admin login failed: {status} {body:?}");
    let token = body["data"]["access_token"].as_str().unwrap().to_string();

    let uid = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("Apicfg{uid}");
    let singular = format!("apicfg{uid}");
    let plural = format!("apicfgs{uid}");

    let schema = json!({
        "name": name,
        "singular": singular,
        "plural": plural,
        "table": plural,
        "implements": ["ownable"],
        "fields": [{ "name": "title", "field_type": "text" }]
    });
    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/content-types", schema, &token),
    )
    .await;
    if !status.is_success() {
        let keys: Vec<String> = state
            .content_type_registry
            .all()
            .iter()
            .map(|ct| ct.registry_key())
            .collect();
        panic!("create schema failed: {status} {body:?} registry={keys:?}");
    }

    // Defaults: list/get/create=authed, update=owner, delete=admin
    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/content-types/{singular}"), &token),
    )
    .await;
    assert!(status.is_success(), "get schema failed: {status}");
    assert_eq!(body["data"]["api"]["list"]["access"], "authed");
    assert_eq!(body["data"]["api"]["create"]["access"], "authed");
    assert_eq!(body["data"]["api"]["update"]["access"], "owner");
    assert_eq!(body["data"]["api"]["delete"]["access"], "admin");

    // Update only the api config
    let (status, body) = send(
        &mut app,
        put_json_auth(
            &format!("/api/v1/admin/content-types/{singular}"),
            json!({
                "api": {
                    "list": { "access": "owner", "filter": "status = \"published\"", "cache": true, "fields": ["id", "title"] },
                    "get": { "access": "public" },
                    "create": { "access": "admin" },
                    "update": { "access": "owner" },
                    "delete": { "access": "admin" }
                }
            }),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "update api failed: {status} {body:?}");

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/content-types/{singular}"), &token),
    )
    .await;
    assert!(
        status.is_success(),
        "get schema after update failed: {status}"
    );
    assert_eq!(body["data"]["api"]["list"]["access"], "owner");
    assert_eq!(
        body["data"]["api"]["list"]["filter"],
        "status = \"published\""
    );
    assert_eq!(body["data"]["api"]["list"]["cache"], true);
    assert_eq!(body["data"]["api"]["list"]["fields"][0], "id");
    assert_eq!(body["data"]["api"]["get"]["access"], "public");
    assert_eq!(body["data"]["api"]["create"]["access"], "admin");

    let _ = std::fs::remove_file(&db_path);
}

// ── Integration Plane: push pipeline end-to-end ─────────────────────

#[tokio::test]
async fn integration_ingress_push_end_to_end() {
    let (mut app, state) = test_app().await;

    // 1. Target content type (table created via migrate + registered).
    let toml = r#"
[content_type]
name = "Ingress Note"
singular = "ingress_note"
plural = "ingress_notes"
table = "ingress_notes"

[fields.external_id]
type = "text"

[fields.body]
type = "text"
"#;
    let schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await.unwrap();
    state
        .content_type_registry
        .register(
            schema,
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    // 2. Channel (challenge verify: guards GET only; POST passes verify).
    let plane = state.integration.as_ref().unwrap();
    let channel = raisfast::integration::ItgChannel {
        id: raisfast::utils::id::new_snowflake_id(),
        tenant_id: "default".into(),
        channel_key: "e2e-notes".into(),
        provider: "generic-hmac".into(),
        display_name: "E2E".into(),
        mode: "push".into(),
        transport: "http1".into(),
        framing: "raw".into(),
        codec: "json".into(),
        endpoint: None,
        verify_kind: "challenge".into(),
        verify_config: None,
        credentials: None,
        mapping: Some(json!({
            "external_id": "$.id",
            "kind": "const:Message",
            "payload": { "body": "$.text" }
        })),
        normalizer_plugin: None,
        pull_semantics: None,
        pull_config: None,
        ack_kind: "http-200".into(),
        redelivery_max: 5,
        backpressure: None,
        target_type: "ingress_note".into(),
        route_extra: Some(json!({ "jobs": [ { "job_type": "ingress.e2e.noop" } ] })),
        status: "idle".into(),
        last_error: None,
        lease_owner: None,
        enabled: true,
        version: 1,
        shadow: false,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    };
    raisfast::integration::channel::model::insert(&state.pool, &channel)
        .await
        .unwrap();
    plane.channels().refresh().await.unwrap();

    // 3. First push → delivered (receipt + CT row + steps + pending job slot).
    let body = json!({"id": "m-001", "text": "hello plane"});
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/e2e-notes", body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first push should be acked 200");

    let receipts: Vec<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT id, status, steps FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_all(&state.pool)
            .await
            .unwrap();
    assert_eq!(receipts.len(), 1, "exactly one receipt");
    assert_eq!(receipts[0].1, "delivered");
    let steps: Value =
        serde_json::from_str(receipts[0].2.as_deref().unwrap_or("[]")).unwrap_or(Value::Null);
    let names: Vec<&str> = steps
        .as_array()
        .map(|a| a.iter().filter_map(|s| s["step"].as_str()).collect())
        .unwrap_or_default();
    for expected in ["verify", "normalize", "dedup", "route", "ack"] {
        assert!(names.contains(&expected), "steps missing '{expected}': {names:?}");
    }
    assert!(
        names.iter().any(|n| n.starts_with("job:")),
        "pending job placeholder present: {names:?}"
    );

    let ct_row: Option<(i64, String, String)> =
        sqlx::query_as("SELECT id, external_id, body FROM ingress_notes WHERE external_id = ?")
            .bind("m-001")
            .fetch_optional(&state.pool)
            .await
            .unwrap();
    let ct_row = ct_row.expect("CT row written");
    assert_eq!(ct_row.2, "hello plane");
    assert_eq!(ct_row.1, "m-001");

    // 4. Repost the same body → duplicate, no second receipt/CT row.
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/e2e-notes", body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "duplicate must also be acked 200");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "dedup keeps exactly one receipt");

    let ct_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ingress_notes WHERE external_id = ?")
            .bind("m-001")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(ct_count, 1, "dedup keeps exactly one CT row");

    // 5. GET challenge echo handshake.
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/ingress/e2e-notes?echostr=handshake-42")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK, "challenge handshake echoes 200");

    // 6. Unknown channel → 404.
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/no-such-channel", json!({"id": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn integration_internal_retry_roundtrip() {
    let (mut app, state) = test_app().await;

    // Target CT whose required field is NEVER provided by the mapping →
    // route fails validation → internal retry state machine kicks in.
    let toml = r#"
[content_type]
name = "Retry Note"
singular = "retry_note"
plural = "retry_notes"
table = "retry_notes"

[fields.external_id]
type = "text"

[fields.mandatory]
type = "text"
required = true
"#;
    let schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await.unwrap();
    state
        .content_type_registry
        .register(
            schema,
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    let plane = state.integration.as_ref().unwrap();
    let channel = raisfast::integration::ItgChannel {
        id: raisfast::utils::id::new_snowflake_id(),
        tenant_id: "default".into(),
        channel_key: "retry-ch".into(),
        provider: "generic-hmac".into(),
        display_name: "Retry".into(),
        mode: "push".into(),
        transport: "http1".into(),
        framing: "raw".into(),
        codec: "json".into(),
        endpoint: None,
        verify_kind: "challenge".into(),
        verify_config: None,
        credentials: None,
        mapping: Some(json!({
            "external_id": "$.id",
            "payload": { "external_id": "$.id" }   // mandatory 缺失 → route 校验失败
        })),
        normalizer_plugin: None,
        pull_semantics: None,
        pull_config: None,
        ack_kind: "http-200".into(),
        redelivery_max: 2,
        backpressure: None,
        target_type: "retry_note".into(),
        route_extra: None,
        status: "idle".into(),
        last_error: None,
        lease_owner: None,
        enabled: true,
        version: 1,
        shadow: false,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    };
    raisfast::integration::channel::model::insert(&state.pool, &channel)
        .await
        .unwrap();
    plane.channels().refresh().await.unwrap();
    raisfast::integration::set_shared_plane(state.integration.clone().unwrap());

    // 1. First push → route fails (missing required) → retrying, ack 200.
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/retry-ch", json!({"id": "r-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "internal mode acks 200 on failure");

    let row: (String, i64) =
        sqlx::query_as("SELECT status, attempts FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(row.0, "retrying", "first failure → retrying");
    assert_eq!(row.1, 1, "attempts = 1");
    let trace_id: i64 =
        sqlx::query_scalar("SELECT id FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_one(&state.pool)
            .await
            .unwrap();

    // 2. Simulate the retry job: still failing → attempts=2, still retrying.
    let pipeline = plane.pipeline();
    let res = pipeline.run_retry(trace_id).await.unwrap();
    assert_eq!(res, raisfast::integration::pipeline::RetryResult::Rescheduled);
    let row: (String, i64) =
        sqlx::query_as("SELECT status, attempts FROM itg_receipts WHERE id = ?")
            .bind(trace_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!((row.0.as_str(), row.1), ("retrying", 2));

    // 3. Retry again → exceeds redelivery_max=2 → dead + steps record.
    let res = pipeline.run_retry(trace_id).await.unwrap();
    assert_eq!(res, raisfast::integration::pipeline::RetryResult::Dead);
    let status_str: String =
        sqlx::query_scalar("SELECT status FROM itg_receipts WHERE id = ?")
            .bind(trace_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(status_str, "dead");
}

#[tokio::test]
async fn integration_retry_recovers_when_target_appears() {
    let (mut app, state) = test_app().await;
    let plane = state.integration.as_ref().unwrap();

    // Channel targeting a CT that does not exist yet → route fails.
    let channel = raisfast::integration::ItgChannel {
        id: raisfast::utils::id::new_snowflake_id(),
        tenant_id: "default".into(),
        channel_key: "recover-ch".into(),
        provider: "generic-hmac".into(),
        display_name: "Recover".into(),
        mode: "push".into(),
        transport: "http1".into(),
        framing: "raw".into(),
        codec: "json".into(),
        endpoint: None,
        verify_kind: "challenge".into(),
        verify_config: None,
        credentials: None,
        mapping: Some(json!({"external_id": "$.id", "payload": {"body": "$.text"}})),
        normalizer_plugin: None,
        pull_semantics: None,
        pull_config: None,
        ack_kind: "http-200".into(),
        redelivery_max: 5,
        backpressure: None,
        target_type: "recover_note".into(),
        route_extra: None,
        status: "idle".into(),
        last_error: None,
        lease_owner: None,
        enabled: true,
        version: 1,
        shadow: false,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    };
    raisfast::integration::channel::model::insert(&state.pool, &channel)
        .await
        .unwrap();
    plane.channels().refresh().await.unwrap();

    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/recover-ch", json!({"id": "rc-1", "text": "later"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let trace_id: i64 =
        sqlx::query_scalar("SELECT id FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_one(&state.pool)
            .await
            .unwrap();

    // Target CT appears → retry succeeds → delivered + CT row + steps merged.
    let toml = r#"
[content_type]
name = "Recover Note"
singular = "recover_note"
plural = "recover_notes"
table = "recover_notes"

[fields.external_id]
type = "text"

[fields.body]
type = "text"
"#;
    let schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await.unwrap();
    state
        .content_type_registry
        .register(
            schema,
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    let res = plane.pipeline().run_retry(trace_id).await.unwrap();
    assert_eq!(res, raisfast::integration::pipeline::RetryResult::Delivered);
    let status_str: String =
        sqlx::query_scalar("SELECT status FROM itg_receipts WHERE id = ?")
            .bind(trace_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(status_str, "delivered");

    let body: String =
        sqlx::query_scalar("SELECT body FROM recover_notes WHERE external_id = ?")
            .bind("rc-1")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(body, "later");

    // Normalize ran exactly once (snapshot determinism): no counter available,
    // but envelope snapshot must equal the first pass payload.
    let env: String =
        sqlx::query_scalar("SELECT CAST(envelope AS TEXT) FROM itg_receipts WHERE id = ?")
            .bind(trace_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(env.contains("rc-1"), "snapshot persisted");
}

#[tokio::test]
async fn integration_pending_flip_and_append_step() {
    let (mut app, state) = test_app().await;
    let plane = state.integration.as_ref().unwrap();

    let channel = raisfast::integration::ItgChannel {
        id: raisfast::utils::id::new_snowflake_id(),
        tenant_id: "default".into(),
        channel_key: "flip-ch".into(),
        provider: "generic-hmac".into(),
        display_name: "Flip".into(),
        mode: "push".into(),
        transport: "http1".into(),
        framing: "raw".into(),
        codec: "json".into(),
        endpoint: None,
        verify_kind: "challenge".into(),
        verify_config: None,
        credentials: None,
        mapping: Some(json!({"external_id": "$.id", "payload": {"body": "$.text"}})),
        normalizer_plugin: None,
        pull_semantics: None,
        pull_config: None,
        ack_kind: "http-200".into(),
        redelivery_max: 5,
        backpressure: None,
        target_type: "ingress_note".into(),
        route_extra: Some(json!({"jobs": [{"job_type": "flip.echo"}]})),
        status: "idle".into(),
        last_error: None,
        lease_owner: None,
        enabled: true,
        version: 1,
        shadow: false,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    };
    raisfast::integration::channel::model::insert(&state.pool, &channel)
        .await
        .unwrap();
    plane.channels().refresh().await.unwrap();

    let toml = r#"
[content_type]
name = "Ingress Note"
singular = "ingress_note"
plural = "ingress_notes"
table = "ingress_notes"

[fields.external_id]
type = "text"

[fields.body]
type = "text"
"#;
    let schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await.unwrap();
    state
        .content_type_registry
        .register(
            schema,
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/flip-ch", json!({"id": "f-1", "text": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let trace_id: i64 =
        sqlx::query_scalar("SELECT id FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_one(&state.pool)
            .await
            .unwrap();

    // Pending placeholder exists.
    let steps: String =
        sqlx::query_scalar("SELECT CAST(steps AS TEXT) FROM itg_receipts WHERE id = ?")
            .bind(trace_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(steps.contains("\"job:flip.echo\"") && steps.contains("\"pending\""));

    // Flip to terminal + append a manual entry.
    use raisfast::types::snowflake_id::SnowflakeId;
    raisfast::integration::receipt::flip_pending_step(
        &state.pool,
        SnowflakeId::new(trace_id),
        "flip.echo",
        true,
        "done in 3ms",
    )
    .await
    .unwrap();
    raisfast::integration::receipt::append_step(
        &state.pool,
        SnowflakeId::new(trace_id),
        &serde_json::json!({"step": "egress:test.op#1", "status": "ok", "ms": 5}),
    )
    .await
    .unwrap();

    let steps: String =
        sqlx::query_scalar("SELECT CAST(steps AS TEXT) FROM itg_receipts WHERE id = ?")
            .bind(trace_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    let parsed: Value = serde_json::from_str(&steps).unwrap();
    let arr = parsed.as_array().unwrap();
    let flip = arr
        .iter()
        .find(|s| s["step"] == "job:flip.echo")
        .expect("flip entry");
    assert_eq!(flip["status"], "ok");
    assert_eq!(flip["detail"], "done in 3ms");
    assert!(arr.iter().any(|s| s["step"] == "egress:test.op#1"));
}

#[tokio::test]
async fn integration_http_pull_cursor_increments() {
    use std::sync::Mutex;
    // Mock upstream: GET /items?since_id=&limit= → ids > since_id, asc, capped.
    static ITEMS: Mutex<Vec<i64>> = Mutex::new(Vec::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    async fn mock_items(
        Query(q): Query<std::collections::HashMap<String, String>>,
    ) -> axum::Json<Value> {
        let since: i64 = q.get("since_id").and_then(|s| s.parse().ok()).unwrap_or(0);
        let limit: usize = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);
        let items: Vec<Value> = ITEMS
            .lock()
            .unwrap()
            .iter()
            .filter(|id| **id > since)
            .take(limit)
            .map(|id| json!({"id": id.to_string(), "text": format!("msg-{id}")}))
            .collect();
        axum::Json(json!({"items": items}))
    }

    tokio::spawn(async move {
        let app = axum::Router::new().route("/items", get(mock_items));
        axum::serve(listener, app).await.unwrap();
    });

    let (mut app, state) = test_app().await;
    let _ = &mut app; // keep harness consistent
    let plane = state.integration.as_ref().unwrap();

    // Target CT.
    let toml = r#"
[content_type]
name = "Ingress Note"
singular = "ingress_note"
plural = "ingress_notes"
table = "ingress_notes"

[fields.external_id]
type = "text"

[fields.body]
type = "text"
"#;
    let schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await.unwrap();
    state
        .content_type_registry
        .register(
            schema,
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    // Pull channel.
    let channel = raisfast::integration::ItgChannel {
        id: raisfast::utils::id::new_snowflake_id(),
        tenant_id: "default".into(),
        channel_key: "pull-ch".into(),
        provider: "generic-rest".into(),
        display_name: "Pull".into(),
        mode: "pull".into(),
        transport: "http1".into(),
        framing: "raw".into(),
        codec: "json".into(),
        endpoint: Some(format!("http://{addr}/items")),
        verify_kind: "none".into(),
        verify_config: None,
        credentials: None,
        mapping: Some(json!({"external_id": "$.id", "payload": {"body": "$.text"}})),
        normalizer_plugin: None,
        pull_semantics: Some("cursor".into()),
        pull_config: Some(json!({
            "list_path": "$.items", "id_field": "id",
            "param": "since_id", "page_size": 2, "max_pages": 10
        })),
        ack_kind: "none".into(),
        redelivery_max: 5,
        backpressure: None,
        target_type: "ingress_note".into(),
        route_extra: None,
        status: "idle".into(),
        last_error: None,
        lease_owner: None,
        enabled: true,
        version: 1,
        shadow: false,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    };
    raisfast::integration::channel::model::insert(&state.pool, &channel)
        .await
        .unwrap();
    plane.channels().refresh().await.unwrap();

    // ── Run 1: three items across two pages ────────────────────────────
    ITEMS.lock().unwrap().extend([1, 2, 3]);
    let s = raisfast::integration::connector::http_pull::run(
        &state.pool,
        plane.pipeline(),
        &channel,
        None,
    )
    .await
    .unwrap();
    assert_eq!((s.fetched, s.delivered, s.duplicates, s.failed), (3, 3, 0, 0));
    assert_eq!(s.pages, 2, "page_size=2 → two pages");

    let cursor: String = sqlx::query_scalar(
        "SELECT CAST(cursor_value AS TEXT) FROM itg_channel_cursors WHERE channel_id = ?",
    )
    .bind(*channel.id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert!(cursor.contains("\"since_id\":\"3\""), "cursor at last id: {cursor}");

    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingress_notes")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(n, 3);

    // ── Run 2: incremental (two new items only) ────────────────────────
    ITEMS.lock().unwrap().extend([4, 5]);
    let s = raisfast::integration::connector::http_pull::run(
        &state.pool,
        plane.pipeline(),
        &channel,
        None,
    )
    .await
    .unwrap();
    assert_eq!((s.fetched, s.delivered, s.duplicates), (2, 2, 0));

    // ── Run 3: no new items → empty fetch ──────────────────────────────
    let s = raisfast::integration::connector::http_pull::run(
        &state.pool,
        plane.pipeline(),
        &channel,
        None,
    )
    .await
    .unwrap();
    assert_eq!(s.fetched, 0);

    // ── Run 4: cursor rewind (simulate lost advance) → duplicates absorbed,
    //    no duplicate CT rows —— 不重不漏 ───────────────────────────────
    sqlx::query("UPDATE itg_channel_cursors SET cursor_value = ? WHERE channel_id = ?")
        .bind(json!({"since_id": "2"}))
        .bind(*channel.id)
        .execute(&state.pool)
        .await
        .unwrap();
    let s = raisfast::integration::connector::http_pull::run(
        &state.pool,
        plane.pipeline(),
        &channel,
        None,
    )
    .await
    .unwrap();
    assert_eq!(s.fetched, 3);
    assert_eq!(s.duplicates, 3, "rewind re-fetches are all duplicates");
    assert_eq!(s.delivered, 0);

    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingress_notes")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(n, 5, "still exactly five CT rows");
    let receipts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(receipts, 5, "five receipts, no duplicates rows");
}

#[tokio::test]
async fn integration_raw_archive_and_replay() {
    let (mut app, state) = test_app().await;
    let plane = state.integration.as_ref().unwrap();

    // Target CT (external_id association present → replay-capable).
    let toml = r#"
[content_type]
name = "Ingress Note"
singular = "ingress_note"
plural = "ingress_notes"
table = "ingress_notes"

[fields.external_id]
type = "text"
unique = true

[fields.body]
type = "text"
"#;
    let schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await.unwrap();
    state
        .content_type_registry
        .register(
            schema,
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    let channel = raisfast::integration::ItgChannel {
        id: raisfast::utils::id::new_snowflake_id(),
        tenant_id: "default".into(),
        channel_key: "archive-ch".into(),
        provider: "generic-hmac".into(),
        display_name: "Archive".into(),
        mode: "push".into(),
        transport: "http1".into(),
        framing: "raw".into(),
        codec: "json".into(),
        endpoint: None,
        verify_kind: "challenge".into(),
        verify_config: None,
        credentials: None,
        mapping: Some(json!({"external_id": "$.id", "payload": {"body": "$.text"}})),
        normalizer_plugin: None,
        pull_semantics: None,
        pull_config: None,
        ack_kind: "http-200".into(),
        redelivery_max: 5,
        backpressure: None,
        target_type: "ingress_note".into(),
        route_extra: None,
        status: "idle".into(),
        last_error: None,
        lease_owner: None,
        enabled: true,
        version: 1,
        shadow: false,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    };
    raisfast::integration::channel::model::insert(&state.pool, &channel)
        .await
        .unwrap();
    plane.channels().refresh().await.unwrap();

    // ── Push → raw archived + raw_ref in snapshot ──────────────────────
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/archive-ch", json!({"id": "a-1", "text": "original"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (trace_id, env_json): (i64, String) =
        sqlx::query_as("SELECT id, CAST(envelope AS TEXT) FROM itg_receipts WHERE channel_id = ?")
            .bind(*channel.id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    let env: Value = serde_json::from_str(&env_json).unwrap();
    let raw_ref = env["raw_ref"].as_str().expect("raw_ref set").to_string();
    assert!(raw_ref.contains("integration/raw"), "path: {raw_ref}");
    assert!(
        tokio::fs::metadata(&raw_ref).await.is_ok(),
        "raw file exists at {raw_ref}"
    );
    let raw = tokio::fs::read(&raw_ref).await.unwrap();
    assert_eq!(raw, br#"{"id":"a-1","text":"original"}"#);

    // ── Corrupt the target row, then replay (upsert) ───────────────────
    sqlx::query("UPDATE ingress_notes SET body = 'stale' WHERE external_id = ?")
        .bind("a-1")
        .execute(&state.pool)
        .await
        .unwrap();

    use raisfast::types::snowflake_id::SnowflakeId;
    let outcome = plane
        .pipeline()
        .run_replay(SnowflakeId::new(trace_id), false)
        .await
        .unwrap();
    match outcome {
        raisfast::integration::pipeline::ReplayOutcome::Upserted { target_id } => {
            assert!(target_id.is_some(), "existing row updated");
        }
        _ => panic!("expected Upserted"),
    }

    let body: String =
        sqlx::query_scalar("SELECT body FROM ingress_notes WHERE external_id = ?")
            .bind("a-1")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(body, "original", "replay restored the snapshot payload");

    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingress_notes")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "upsert does not duplicate rows");

    // steps: original timeline intact + replay#N appended.
    let steps: String =
        sqlx::query_scalar("SELECT CAST(steps AS TEXT) FROM itg_receipts WHERE id = ?")
            .bind(trace_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    let parsed: Value = serde_json::from_str(&steps).unwrap();
    let arr = parsed.as_array().unwrap();
    assert!(
        arr.iter().any(|s| s["step"].as_str().unwrap_or("").starts_with("replay#")),
        "replay appended: {steps}"
    );
    assert!(
        arr.iter().any(|s| s["step"] == "verify"),
        "original timeline preserved"
    );

    // ── Dry-run: report only, zero writes ──────────────────────────────
    let outcome = plane
        .pipeline()
        .run_replay(SnowflakeId::new(trace_id), true)
        .await
        .unwrap();
    match outcome {
        raisfast::integration::pipeline::ReplayOutcome::DryRun { report } => {
            assert_eq!(report["external_id"], "a-1");
            assert!(report["would_write"]["body"] == "original", "report carries snapshot payload");
        }
        _ => panic!("expected DryRun"),
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingress_notes")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "dry-run wrote nothing");
}

#[tokio::test]
async fn integration_admin_channels_and_receipts_api() {
    let (mut app, state) = test_app().await;
    let _ = create_admin(&state.pool).await;
    let (status, body) = send(
        &mut app,
        post_json(
            "/api/v1/auth/login",
            json!({ "email": ADMIN_EMAIL.with(|c| c.borrow().clone()), "password": "AdminPass123!" }),
        ),
    )
    .await;
    assert!(status.is_success(), "admin login failed: {status} {body:?}");
    let token = body["data"]["access_token"].as_str().unwrap().to_string();

    // Target CT.
    let toml = r#"
[content_type]
name = "Ingress Note"
singular = "ingress_note"
plural = "ingress_notes"
table = "ingress_notes"

[fields.external_id]
type = "text"

[fields.body]
type = "text"
"#;
    let schema =
        raisfast::content_type::schema::ContentTypeSchema::parse_from_str(toml).unwrap();
    let repo = raisfast::content_type::repository::ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await.unwrap();
    state
        .content_type_registry
        .register(
            schema,
            &state.config.rule_engine,
            &state.config.builtins.reserved_route_segments(),
            &state.protocol_registry.names(),
            &state.protocol_registry,
        )
        .unwrap();

    // ── Create channel via admin API ────────────────────────────────
    let create = json!({
        "channel_key": "admin-ch",
        "provider": "generic-hmac",
        "mode": "push", "transport": "http1", "framing": "raw", "codec": "json",
        "verify_kind": "challenge",
        "mapping": {"external_id": "$.id", "payload": {"body": "$.text"}},
        "target_type": "ingress_note",
    });
    let (status, body) = send(
        &mut app,
        post_json_auth("/api/v1/admin/integration/channels", create.clone(), &token),
    )
    .await;
    assert!(status.is_success(), "create failed: {status} {body:?}");
    let channel_id = body["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["has_credentials"], false);

    // Duplicate active key rejected.
    let (status, _) = send(
        &mut app,
        post_json_auth("/api/v1/admin/integration/channels", create, &token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "duplicate key rejected");

    // Bad stack rejected.
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/integration/channels",
            json!({
                "channel_key": "bad-ch", "provider": "x",
                "mode": "stream", "transport": "ws", "framing": "raw", "codec": "json",
                "verify_kind": "none", "target_type": "ingress_note"
            }),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "stream rejected in this phase");

    // ── test-mapping preview (zero writes) ───────────────────────────
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/integration/channels/{channel_id}/test-mapping"),
            json!({"sample": r#"{"id":"t-1","text":"preview"}"#}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "test-mapping failed: {body:?}");
    assert_eq!(body["data"]["matched"], true);
    assert_eq!(body["data"]["external_id"], "t-1");
    assert_eq!(body["data"]["payload"]["body"], "preview");

    // ── Push through the created channel, then query receipts ────────
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/admin-ch", json!({"id": "adm-1", "text": "via admin api"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "routed through admin-created channel");

    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("GET")
            .uri("/api/v1/admin/integration/receipts?status=delivered")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(status.is_success(), "receipts list: {body:?}");
    let items = body["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "filtered list");
    let trace_id = items[0]["id"].as_i64().unwrap();

    // Detail: envelope + steps timeline.
    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("GET")
            .uri(format!("/api/v1/admin/integration/receipts/{trace_id}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["external_id"], "adm-1");
    assert!(body["data"]["steps"].is_array());

    // Trace: first pass + no pending → complete.
    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("GET")
            .uri(format!("/api/v1/admin/integration/receipts/{trace_id}/trace"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["complete"], true, "no pending jobs declared");
    assert!(body["data"]["first_pass"].as_array().unwrap().len() >= 5);

    // ── Update (mapping change) + delete ─────────────────────────────
    let (status, _) = send(
        &mut app,
        Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/admin/integration/channels/{channel_id}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_string(&json!({"display_name": "Renamed"})).unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert!(status.is_success(), "update failed");

    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("GET")
            .uri("/api/v1/admin/integration/channels")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(status.is_success());
    let names: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["display_name"].as_str().unwrap_or(""))
        .collect();
    assert!(names.contains(&"Renamed"));

    let (status, _) = send(
        &mut app,
        Request::builder()
            .method("DELETE")
            .uri(format!("/api/v1/admin/integration/channels/{channel_id}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(status.is_success(), "delete failed");

    // Ingress for the deleted channel → 404.
    let (status, _) = send(
        &mut app,
        post_json("/api/v1/ingress/admin-ch", json!({"id": "adm-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "deleted channel no longer routes");

    // Unauthenticated admin access → forbidden.
    let (status, _) = send(
        &mut app,
        Request::builder()
            .method("GET")
            .uri("/api/v1/admin/integration/channels")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "anonymous blocked");
}

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;

use crate::dto::payment::*;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::pagination::PaginationParams;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/payment/channels/available",
        get,
        list_available_channels_handler,
        "system public",
        "payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/payment/orders",
        get,
        list_user_orders,
        "system authed",
        "payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/payment/orders",
        create,
        create_payment_order_handler,
        "system authed",
        "payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/payment/orders/{id}",
        get,
        get_payment_order_handler,
        "system authed",
        "payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/payment/orders/{id}/cancel",
        post,
        cancel_payment_order_handler,
        "system authed",
        "payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/payment/orders/{id}/transactions",
        get,
        list_order_transactions,
        "system authed",
        "payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/payment/orders/{id}/refunds",
        get,
        list_order_refunds,
        "system authed",
        "payment"
    );
    let r = {
        let mr = axum::routing::post(handle_callback).layer(axum::middleware::from_fn(
            crate::middleware::rate_limit::payment_callback_rate_limit,
        ));
        r.route("/payment/callback/{channel_id}", mr)
    };
    registry.record(
        "POST",
        "/api/v1/payment/callback/{channel_id}",
        "system public",
        "payment",
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/channels",
        get,
        admin_list_channels,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/channels",
        create,
        admin_create_channel,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/channels/{id}",
        get,
        admin_get_channel,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/channels/{id}",
        put,
        admin_update_channel,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/channels/{id}",
        delete,
        admin_delete_channel,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/orders",
        get,
        admin_list_orders,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/orders/{id}",
        get,
        admin_get_order,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/orders/{id}/refund",
        post,
        admin_refund_order,
        "system admin",
        "admin/payment"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/transactions",
        get,
        admin_list_transactions,
        "system admin",
        "admin/payment"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/payment/refunds",
        get,
        admin_list_refunds,
        "system admin",
        "admin/payment"
    )
}

fn to_order_response(o: crate::models::payment_order::PaymentOrder) -> PaymentOrderResponse {
    PaymentOrderResponse {
        id: o.id.to_string(),
        order_id: o.order_id,
        title: o.title,
        amount: o.amount,
        currency: o.currency,
        provider: o.provider,
        provider_order_id: o.provider_order_id,
        provider_method: o.provider_method,
        status: o.status,
        return_url: o.return_url,
        version: o.version,
        provider_data: o.provider_data,
        client_ip: o.client_ip,
        client_language: o.client_language,
        client_country: o.client_country,
        client_user_agent: o.client_user_agent,
        channel_selected_by: o.channel_selected_by,
        metadata: o.metadata,
        redirect_url: None,
        qr_code: None,
        client_secret: None,
        paid_at: o.paid_at.map(|t| t.to_string()),
        cancelled_at: o.cancelled_at.map(|t| t.to_string()),
        expired_at: o.expired_at.map(|t| t.to_string()),
        created_at: o.created_at.to_string(),
        updated_at: o.updated_at.to_string(),
    }
}

fn to_order_response_with_provider(
    o: crate::models::payment_order::PaymentOrder,
    pr: Option<crate::payment::ProviderResponse>,
) -> PaymentOrderResponse {
    let mut resp = to_order_response(o);
    if let Some(pr) = pr {
        resp.redirect_url = pr.redirect_url;
        resp.qr_code = pr.qr_code;
        resp.client_secret = pr.client_secret;
    }
    resp
}

#[utoipa::path(post, path = "/payment/orders", tag = "payments",
    security(("bearer_auth" = [])),
    request_body = CreatePaymentOrderRequest,
    responses((status = 200, description = "Payment order created"))
)]
pub async fn create_payment_order_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePaymentOrderRequest>,
) -> AppResult<ApiResponse<PaymentOrderResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    validation::validate(&req)?;
    let client_ip = extract_client_ip(&headers);
    let client_language = extract_accept_language(&headers);
    let client_user_agent = extract_user_agent(&headers);
    let (order, provider_resp) = state
        .payment_service
        .create_payment_order(
            &auth,
            user_id,
            req,
            client_ip.as_deref(),
            client_language.as_deref(),
            client_user_agent.as_deref(),
        )
        .await?;
    Ok(ApiResponse::success(to_order_response_with_provider(
        order,
        provider_resp,
    )))
}

#[utoipa::path(get, path = "/payment/orders", tag = "payments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "User payment orders"))
)]
pub async fn list_user_orders(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentOrderResponse>>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    params.sanitize();
    let (orders, total) = state
        .payment_service
        .list_user_payment_orders(&auth, user_id, params.page, params.page_size)
        .await?;
    let responses: Vec<PaymentOrderResponse> = orders.into_iter().map(to_order_response).collect();
    Ok(params.paginate(responses, total))
}

#[utoipa::path(get, path = "/payment/orders/{id}", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Payment order ID")),
    responses((status = 200, description = "Payment order detail"))
)]
pub async fn get_payment_order_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PaymentOrderResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let order = state
        .payment_service
        .get_payment_order(&auth, user_id, id)
        .await?;
    Ok(ApiResponse::success(to_order_response(order)))
}

#[utoipa::path(post, path = "/payment/orders/{id}/cancel", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Payment order ID")),
    responses((status = 200, description = "Payment order cancelled"))
)]
pub async fn cancel_payment_order_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .payment_service
        .cancel_payment_order(&auth, id, user_id)
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(get, path = "/payment/orders/{id}/transactions", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Payment order ID")),
    responses((status = 200, description = "Payment transactions"))
)]
pub async fn list_order_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Vec<PaymentTransactionResponse>>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let txs = state
        .payment_service
        .list_order_transactions(&auth, user_id, id)
        .await?;
    let responses: Vec<PaymentTransactionResponse> = txs.into_iter().map(Into::into).collect();
    Ok(ApiResponse::success(responses))
}

#[utoipa::path(get, path = "/payment/orders/{id}/refunds", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Payment order ID")),
    responses((status = 200, description = "Payment refunds"))
)]
pub async fn list_order_refunds(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Vec<PaymentRefundResponse>>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let refunds = state
        .payment_service
        .list_order_refunds(&auth, user_id, id)
        .await?;
    let responses: Vec<PaymentRefundResponse> = refunds.into_iter().map(Into::into).collect();
    Ok(ApiResponse::success(responses))
}

#[utoipa::path(post, path = "/payment/callback/{channel_id}", tag = "payments",
    params(("channel_id" = String, Path, description = "Channel ID")),
    request_body = String,
    responses((status = 200, description = "Callback processed"))
)]
pub async fn handle_callback(
    State(state): State<crate::AppState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<ApiResponse<()>> {
    state
        .payment_service
        .handle_callback(&channel_id, &headers, &body)
        .await?;
    Ok(ApiResponse::success(()))
}

fn extract_client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("X-Real-IP")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
}

fn extract_accept_language(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Accept-Language")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
}

fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get("User-Agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[utoipa::path(get, path = "/payment/channels/available", tag = "payments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Available payment channels"))
)]
pub async fn list_available_channels_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AvailableChannelsQuery>,
) -> AppResult<ApiResponse<AvailableChannelsResponse>> {
    let result = state
        .payment_service
        .list_available_channels(
            &auth,
            &query.order_id,
            query.country.as_deref(),
            query.language.as_deref(),
        )
        .await?;
    Ok(ApiResponse::success(result))
}

#[utoipa::path(get, path = "/admin/payment/channels", tag = "payments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin payment channels list"))
)]
pub async fn admin_list_channels(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentChannelResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (channels, total) = state
        .payment_service
        .list_admin_channels(&auth, params.page, params.page_size)
        .await?;
    let responses: Vec<PaymentChannelResponse> = channels.into_iter().map(Into::into).collect();
    Ok(params.paginate(responses, total))
}

#[utoipa::path(post, path = "/admin/payment/channels", tag = "payments",
    security(("bearer_auth" = [])),
    request_body = CreatePaymentChannelRequest,
    responses((status = 200, description = "Payment channel created"))
)]
pub async fn admin_create_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreatePaymentChannelRequest>,
) -> AppResult<ApiResponse<PaymentChannelResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let channel = state.payment_service.create_channel(&auth, req).await?;
    Ok(ApiResponse::success(PaymentChannelResponse::from(channel)))
}

#[utoipa::path(get, path = "/admin/payment/channels/{id}", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Channel ID")),
    responses((status = 200, description = "Payment channel detail"))
)]
pub async fn admin_get_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PaymentChannelResponse>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let channel = state.payment_service.get_channel(&auth, id).await?;
    Ok(ApiResponse::success(PaymentChannelResponse::from(channel)))
}

#[utoipa::path(put, path = "/admin/payment/channels/{id}", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Channel ID")),
    request_body = UpdatePaymentChannelRequest,
    responses((status = 200, description = "Payment channel updated"))
)]
pub async fn admin_update_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePaymentChannelRequest>,
) -> AppResult<ApiResponse<PaymentChannelResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let channel = state.payment_service.update_channel(&auth, id, req).await?;
    Ok(ApiResponse::success(PaymentChannelResponse::from(channel)))
}

#[utoipa::path(delete, path = "/admin/payment/channels/{id}", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Channel ID")),
    responses((status = 200, description = "Payment channel deleted"))
)]
pub async fn admin_delete_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.payment_service.delete_channel(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(get, path = "/admin/payment/orders", tag = "payments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin payment orders list"))
)]
pub async fn admin_list_orders(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentOrderResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (orders, total) = state
        .payment_service
        .list_admin_payment_orders(&auth, params.page, params.page_size, None)
        .await?;
    let responses: Vec<PaymentOrderResponse> = orders.into_iter().map(to_order_response).collect();
    Ok(params.paginate(responses, total))
}

#[utoipa::path(get, path = "/admin/payment/orders/{id}", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Payment order ID")),
    responses((status = 200, description = "Payment order detail"))
)]
pub async fn admin_get_order(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PaymentOrderResponse>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let order = state
        .payment_service
        .get_payment_order(&auth, SnowflakeId(0), id)
        .await?;
    Ok(ApiResponse::success(to_order_response(order)))
}

#[utoipa::path(post, path = "/admin/payment/orders/{id}/refund", tag = "payments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Payment order ID")),
    request_body = CreateRefundRequest,
    responses((status = 200, description = "Payment order refunded"))
)]
pub async fn admin_refund_order(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateRefundRequest>,
) -> AppResult<ApiResponse<PaymentRefundResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let refund = state
        .payment_service
        .refund_payment_order(&auth, id, req)
        .await?;
    Ok(ApiResponse::success(PaymentRefundResponse::from(refund)))
}

#[utoipa::path(get, path = "/admin/payment/transactions", tag = "payments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin transactions list"))
)]
pub async fn admin_list_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentTransactionResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (txs, total) = state
        .payment_service
        .list_admin_transactions(&auth, params.page, params.page_size)
        .await?;
    let responses: Vec<PaymentTransactionResponse> = txs.into_iter().map(Into::into).collect();
    Ok(params.paginate(responses, total))
}

#[utoipa::path(get, path = "/admin/payment/refunds", tag = "payments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin refunds list"))
)]
pub async fn admin_list_refunds(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentRefundResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (refunds, total) = state
        .payment_service
        .list_admin_refunds(&auth, params.page, params.page_size)
        .await?;
    let responses: Vec<PaymentRefundResponse> = refunds.into_iter().map(Into::into).collect();
    Ok(params.paginate(responses, total))
}

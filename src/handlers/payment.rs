use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post as http_post};

use crate::audit::AuditService;
use crate::dto::payment::*;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::payment;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    let r = axum::Router::new();
    let r = crate::reg_route!(
        r,
        registry,
        "/payment/channels/available",
        get(list_available_channels_handler),
        "system public",
        "payment",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/payment/orders",
        get(list_user_orders).post(create_payment_order_handler),
        "system public",
        "payment",
        ["GET", "POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/payment/orders/{id}",
        get(get_payment_order_handler),
        "system public",
        "payment",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/payment/orders/{id}/cancel",
        http_post(cancel_payment_order_handler),
        "system public",
        "payment",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/payment/orders/{id}/transactions",
        get(list_order_transactions),
        "system public",
        "payment",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/payment/orders/{id}/refunds",
        get(list_order_refunds),
        "system public",
        "payment",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/payment/callback/{channel_doc_id}",
        http_post(handle_callback).layer(axum::middleware::from_fn(crate::middleware::rate_limit::payment_callback_rate_limit)),
        "system public",
        "payment",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/payment/channels",
        get(admin_list_channels).post(admin_create_channel),
        "system admin",
        "admin/payment",
        ["GET", "POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/payment/channels/{id}",
        get(admin_get_channel)
            .put(admin_update_channel)
            .delete(admin_delete_channel),
        "system admin",
        "admin/payment",
        ["GET", "PUT", "DELETE"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/payment/orders",
        get(admin_list_orders),
        "system admin",
        "admin/payment",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/payment/orders/{id}",
        get(admin_get_order),
        "system admin",
        "admin/payment",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/payment/orders/{id}/refund",
        http_post(admin_refund_order),
        "system admin",
        "admin/payment",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/payment/transactions",
        get(admin_list_transactions),
        "system admin",
        "admin/payment",
        ["GET"]
    );
    crate::reg_route!(
        r,
        registry,
        "/admin/payment/refunds",
        get(admin_list_refunds),
        "system admin",
        "admin/payment",
        ["GET"]
    )
}

fn to_order_response(o: crate::models::payment_order::PaymentOrder) -> PaymentOrderResponse {
    PaymentOrderResponse {
        id: o.document_id,
        user_id: o.user_id,
        order_id: o.order_id,
        title: o.title,
        amount: o.amount,
        currency: o.currency,
        channel_id: o.channel_id.to_string(),
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

pub async fn create_payment_order_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePaymentOrderRequest>,
) -> AppResult<ApiResponse<PaymentOrderResponse>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
    validation::validate(&req)?;
    let client_ip = extract_client_ip(&headers);
    let client_language = extract_accept_language(&headers);
    let client_user_agent = extract_user_agent(&headers);
    let (order, provider_resp) = payment::create_payment_order(
        &state.pool,
        state.payment_channel_repo.as_ref(),
        state.payment_order_repo.as_ref(),
        state.product_repo.as_ref(),
        state.order_repo.as_ref(),
        &auth,
        user_int_id,
        req,
        &state.config,
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

pub async fn list_user_orders(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentOrderResponse>>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
    params.sanitize();
    let (orders, total) = payment::list_user_payment_orders(
        state.payment_order_repo.as_ref(),
        &auth,
        user_int_id,
        params.page,
        params.page_size,
    )
    .await?;
    let responses: Vec<PaymentOrderResponse> = orders.into_iter().map(to_order_response).collect();
    Ok(params.paginate(responses, total))
}

pub async fn get_payment_order_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PaymentOrderResponse>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
    let order = payment::get_payment_order(state.payment_order_repo.as_ref(), &auth, user_int_id, &id).await?;
    Ok(ApiResponse::success(to_order_response(order)))
}

pub async fn cancel_payment_order_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
    let audit = AuditService::new(state.pool.clone());
    payment::cancel_payment_order(
        &state.pool,
        state.payment_order_repo.as_ref(),
        state.payment_channel_repo.as_ref(),
        &auth,
        &audit,
        &state.config,
        &id,
        user_int_id,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

pub async fn list_order_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Vec<PaymentTransactionResponse>>> {
    let _ = auth.ensure_authenticated()?;
    let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
    let order = payment::get_payment_order(state.payment_order_repo.as_ref(), &auth, user_int_id, &id).await?;
    let txs = state
        .payment_tx_repo
        .find_by_payment_order_id(order.id, auth.tenant_id())
        .await?;
    let responses: Vec<PaymentTransactionResponse> = txs.into_iter().map(Into::into).collect();
    Ok(ApiResponse::success(responses))
}

pub async fn list_order_refunds(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Vec<PaymentRefundResponse>>> {
    let _ = auth.ensure_authenticated()?;
    let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
    let order = payment::get_payment_order(state.payment_order_repo.as_ref(), &auth, user_int_id, &id).await?;
    let refunds = state
        .payment_refund_repo
        .find_by_payment_order_id(order.id, auth.tenant_id())
        .await?;
    let responses: Vec<PaymentRefundResponse> = refunds.into_iter().map(Into::into).collect();
    Ok(ApiResponse::success(responses))
}

pub async fn handle_callback(
    State(state): State<crate::AppState>,
    Path(channel_doc_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<ApiResponse<()>> {
    let audit = AuditService::new(state.pool.clone());
    payment::handle_callback(
        &state.pool,
        state.payment_channel_repo.as_ref(),
        state.payment_order_repo.as_ref(),
        state.payment_tx_repo.as_ref(),
        state.wallet_repo.as_ref(),
        &audit,
        &state.config,
        &channel_doc_id,
        &headers,
        &body,
    )
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

pub async fn list_available_channels_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AvailableChannelsQuery>,
) -> AppResult<ApiResponse<AvailableChannelsResponse>> {
    let result = payment::list_available_channels(
        state.payment_channel_repo.as_ref(),
        state.order_repo.as_ref(),
        &auth,
        &query.order_id,
        query.country.as_deref(),
        query.language.as_deref(),
    )
    .await?;
    Ok(ApiResponse::success(result))
}

pub async fn admin_list_channels(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentChannelResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (channels, total) = state
        .payment_channel_repo
        .find_all_admin_paginated(auth.tenant_id(), params.page, params.page_size, None)
        .await?;
    let responses: Vec<PaymentChannelResponse> = channels.into_iter().map(Into::into).collect();
    Ok(params.paginate(responses, total))
}

pub async fn admin_create_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreatePaymentChannelRequest>,
) -> AppResult<ApiResponse<PaymentChannelResponse>> {
    validation::validate(&req)?;
    let audit = AuditService::new(state.pool.clone());
    let channel = payment::create_channel(
        state.payment_channel_repo.as_ref(),
        &auth,
        &state.config,
        &audit,
        req,
    )
    .await?;
    Ok(ApiResponse::success(PaymentChannelResponse::from(channel)))
}

pub async fn admin_get_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PaymentChannelResponse>> {
    let channel = payment::get_channel(state.payment_channel_repo.as_ref(), &auth, &id).await?;
    Ok(ApiResponse::success(PaymentChannelResponse::from(channel)))
}

pub async fn admin_update_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePaymentChannelRequest>,
) -> AppResult<ApiResponse<PaymentChannelResponse>> {
    validation::validate(&req)?;
    let audit = AuditService::new(state.pool.clone());
    let channel = payment::update_channel(
        state.payment_channel_repo.as_ref(),
        &auth,
        &state.config,
        &audit,
        &id,
        req,
    )
    .await?;
    Ok(ApiResponse::success(PaymentChannelResponse::from(channel)))
}

pub async fn admin_delete_channel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let audit = AuditService::new(state.pool.clone());
    payment::delete_channel(state.payment_channel_repo.as_ref(), &auth, &audit, &id).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_list_orders(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentOrderResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (orders, total) = payment::list_admin_payment_orders(
        state.payment_order_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
        None,
    )
    .await?;
    let responses: Vec<PaymentOrderResponse> = orders.into_iter().map(to_order_response).collect();
    Ok(params.paginate(responses, total))
}

pub async fn admin_get_order(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PaymentOrderResponse>> {
    auth.ensure_admin()?;
    let order = payment::get_payment_order(state.payment_order_repo.as_ref(), &auth, 0, &id).await?;
    Ok(ApiResponse::success(to_order_response(order)))
}

pub async fn admin_refund_order(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateRefundRequest>,
) -> AppResult<ApiResponse<PaymentRefundResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let audit = AuditService::new(state.pool.clone());
    let refund = payment::refund_payment_order(
        &state.pool,
        state.payment_order_repo.as_ref(),
        state.payment_channel_repo.as_ref(),
        state.payment_tx_repo.as_ref(),
        state.payment_refund_repo.as_ref(),
        state.wallet_repo.as_ref(),
        &auth,
        &audit,
        &state.config,
        &id,
        req,
    )
    .await?;
    Ok(ApiResponse::success(PaymentRefundResponse::from(refund)))
}

pub async fn admin_list_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentTransactionResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (txs, total) = payment::list_admin_transactions(
        state.payment_tx_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
    )
    .await?;
    let responses: Vec<PaymentTransactionResponse> = txs.into_iter().map(Into::into).collect();
    Ok(params.paginate(responses, total))
}

pub async fn admin_list_refunds(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PaymentRefundResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (refunds, total) = payment::list_admin_refunds(
        state.payment_refund_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
    )
    .await?;
    let responses: Vec<PaymentRefundResponse> = refunds.into_iter().map(Into::into).collect();
    Ok(params.paginate(responses, total))
}

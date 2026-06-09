use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{
    CancelOrderRequest, CreateOrderRequest, OrderItemResponse, OrderResponse, OrderStatsResponse,
    ShipOrderRequest, UpdateAdminRemarkRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;

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
        "/orders",
        get,
        list_orders,
        "system authed",
        "orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/orders",
        create,
        create_order,
        "system authed",
        "orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/orders/{id}",
        get,
        get_order,
        "system authed",
        "orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/orders/{id}",
        put,
        cancel_order_handler,
        "system authed",
        "orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/orders/{id}/confirm",
        post,
        confirm_receipt,
        "system authed",
        "orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/orders",
        get,
        admin_list,
        "system admin",
        "admin/orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/orders/{id}",
        get,
        admin_get,
        "system admin",
        "admin/orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/orders/{id}/pay",
        post,
        admin_pay,
        "system admin",
        "admin/orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/orders/{id}/ship",
        post,
        admin_ship,
        "system admin",
        "admin/orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/orders/{id}/cancel",
        post,
        admin_cancel,
        "system admin",
        "admin/orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/orders/{id}/refund",
        post,
        admin_refund,
        "system admin",
        "admin/orders"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/orders/{id}/remark",
        put,
        admin_update_remark,
        "system admin",
        "admin/orders"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/orders/stats",
        get,
        admin_stats,
        "system admin",
        "admin/orders"
    )
}

fn to_order_response(
    o: crate::models::order::Order,
    items: Vec<crate::models::order_item::OrderItem>,
) -> OrderResponse {
    OrderResponse {
        id: o.id.to_string(),
        order_no: o.order_no,
        subtotal: o.subtotal,
        discount_amount: o.discount_amount,
        shipping_amount: o.shipping_amount,
        total_amount: o.total_amount,
        currency: o.currency,
        status: o.status.to_string(),
        buyer_name: o.buyer_name,
        buyer_phone: o.buyer_phone,
        buyer_email: o.buyer_email,
        shipping_address: o.shipping_address,
        tracking_no: o.tracking_no,
        carrier: o.carrier,
        remark: o.remark,
        admin_remark: o.admin_remark,
        delivery_data: o.delivery_data,
        paid_at: o.paid_at.map(|t| t.to_string()),
        completed_at: o.completed_at.map(|t| t.to_string()),
        cancelled_at: o.cancelled_at.map(|t| t.to_string()),
        created_at: o.created_at.to_string(),
        updated_at: o.updated_at.to_string(),
        items: items.into_iter().map(OrderItemResponse::from).collect(),
    }
}

#[utoipa::path(post, path = "/orders", tag = "orders",
    security(("bearer_auth" = [])),
    request_body = CreateOrderRequest,
    responses((status = 200, description = "Order created"))
)]
pub async fn create_order(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateOrderRequest>,
) -> AppResult<ApiResponse<OrderResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    validation::validate(&req).map_err(|e| {
        tracing::error!("order create validation failed: {e}");
        e
    })?;
    let (o, items) = state
        .order_service
        .create(&auth, user_id, req)
        .await
        .map_err(|e| {
            tracing::error!("order create service failed: {e}");
            e
        })?;
    Ok(ApiResponse::success(to_order_response(o, items)))
}

#[utoipa::path(get, path = "/orders", tag = "orders",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "User orders list"))
)]
pub async fn list_orders(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<OrderResponse>>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    params.sanitize();
    let (orders, total) = state
        .order_service
        .list_user(&auth, user_id, params.page, params.page_size)
        .await?;
    let responses: Vec<_> = orders
        .into_iter()
        .map(|(o, items)| to_order_response(o, items))
        .collect();
    Ok(params.paginate(responses, total))
}

#[utoipa::path(get, path = "/orders/{id}", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    responses((status = 200, description = "Order detail"))
)]
pub async fn get_order(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<OrderResponse>> {
    auth.ensure_authenticated()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let (o, items) = state.order_service.get(&auth, id).await?;
    Ok(ApiResponse::success(to_order_response(o, items)))
}

#[utoipa::path(put, path = "/orders/{id}", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    request_body = CancelOrderRequest,
    responses((status = 200, description = "Order cancelled"))
)]
pub async fn cancel_order_handler(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(_req): Json<CancelOrderRequest>,
) -> AppResult<ApiResponse<()>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.order_service.cancel(&auth, id, user_id).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/orders/{id}/confirm", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    responses((status = 200, description = "Receipt confirmed"))
)]
pub async fn confirm_receipt(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .order_service
        .confirm_receipt(&auth, id, user_id)
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(get, path = "/admin/orders", tag = "orders",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin orders list"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<crate::dto::order::AdminOrderListQuery>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<OrderResponse>>> {
    auth.ensure_admin()?;
    let params = PaginationParams::from_options(query.page, query.page_size);
    let (orders, total) = state
        .order_service
        .list_admin(
            &auth,
            params.page,
            params.page_size,
            query.status.as_deref(),
            query.keyword.as_deref(),
        )
        .await?;
    let responses: Vec<_> = orders
        .into_iter()
        .map(|(o, items)| to_order_response(o, items))
        .collect();
    Ok(params.paginate(responses, total))
}

#[utoipa::path(get, path = "/admin/orders/{id}", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    responses((status = 200, description = "Order detail"))
)]
pub async fn admin_get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<OrderResponse>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let (o, items) = state.order_service.get(&auth, id).await?;
    Ok(ApiResponse::success(to_order_response(o, items)))
}

#[utoipa::path(post, path = "/admin/orders/{id}/ship", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    request_body = ShipOrderRequest,
    responses((status = 200, description = "Order shipped"))
)]
pub async fn admin_ship(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<ShipOrderRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.order_service.ship(&auth, id, &req).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/orders/{id}/cancel", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    responses((status = 200, description = "Order cancelled"))
)]
pub async fn admin_cancel(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.order_service.admin_cancel(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/orders/{id}/pay", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    responses((status = 200, description = "Order marked as paid"))
)]
pub async fn admin_pay(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.order_service.mark_paid(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/orders/{id}/refund", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    responses((status = 200, description = "Order refunded"))
)]
pub async fn admin_refund(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.order_service.refund(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(put, path = "/admin/orders/{id}/remark", tag = "orders",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Order ID")),
    request_body = UpdateAdminRemarkRequest,
    responses((status = 200, description = "Remark updated"))
)]
pub async fn admin_update_remark(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAdminRemarkRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .order_service
        .update_admin_remark(&auth, id, &req.admin_remark)
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(get, path = "/admin/orders/stats", tag = "orders",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Order statistics"))
)]
pub async fn admin_stats(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<OrderStatsResponse>> {
    auth.ensure_admin()?;
    let stats = state.order_service.get_stats(&auth).await?;
    Ok(ApiResponse::success(stats))
}

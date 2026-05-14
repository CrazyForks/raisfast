use axum::Json;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post as http_post, put};

use crate::dto::{
    CancelOrderRequest, CreateOrderRequest, OrderItemResponse, OrderResponse, OrderStatsResponse,
    ShipOrderRequest, UpdateAdminRemarkRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::order;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    let r = axum::Router::new();
    let r = crate::reg_route!(
        r,
        registry,
        "/orders",
        get(list_orders).post(create_order),
        "system public",
        "orders",
        ["GET", "POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/orders/{id}",
        get(get_order).put(cancel_order_handler),
        "system public",
        "orders",
        ["GET", "PUT"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/orders/{id}/confirm",
        http_post(confirm_receipt),
        "system public",
        "orders",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders",
        get(admin_list),
        "system admin",
        "admin/orders",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders/{id}",
        get(admin_get),
        "system admin",
        "admin/orders",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders/{id}/pay",
        http_post(admin_pay),
        "system admin",
        "admin/orders",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders/{id}/ship",
        http_post(admin_ship),
        "system admin",
        "admin/orders",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders/{id}/cancel",
        http_post(admin_cancel),
        "system admin",
        "admin/orders",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders/{id}/refund",
        http_post(admin_refund),
        "system admin",
        "admin/orders",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders/{id}/remark",
        put(admin_update_remark),
        "system admin",
        "admin/orders",
        ["PUT"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/orders/stats",
        get(admin_stats),
        "system admin",
        "admin/orders",
        ["GET"]
    );
    #[allow(clippy::let_and_return)]
    r
}

fn to_order_response(
    o: crate::models::order::Order,
    items: Vec<crate::models::order_item::OrderItem>,
) -> OrderResponse {
    OrderResponse {
        id: o.document_id,
        user_id: o.user_id.to_string(),
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
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_int_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    validation::validate(&req)?;
    let o = order::create_order(
        &state.pool,
        state.product_repo.as_ref(),
        state.order_repo.as_ref(),
        &auth,
        user_int_id,
        req,
    )
    .await?;
    let items = state
        .order_repo
        .find_items_by_order_id(o.id, auth.tenant_id())
        .await?;
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
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_int_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    params.sanitize();
    let (orders, total) = order::list_user_orders(
        state.order_repo.as_ref(),
        &auth,
        user_int_id,
        params.page,
        params.page_size,
    )
    .await?;
    let mut responses = Vec::new();
    for o in orders {
        let items = state
            .order_repo
            .find_items_by_order_id(o.id, auth.tenant_id())
            .await?;
        responses.push(to_order_response(o, items));
    }
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
    let (o, items) = order::get_order(state.order_repo.as_ref(), &auth, &id).await?;
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
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_int_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    order::cancel_order(
        &state.pool,
        state.order_repo.as_ref(),
        &auth,
        &id,
        user_int_id,
    )
    .await?;
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
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_int_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    order::confirm_receipt(
        &state.pool,
        state.order_repo.as_ref(),
        &auth,
        &id,
        user_int_id,
    )
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
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<OrderResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (orders, total) = order::list_admin_orders(
        state.order_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
        None,
    )
    .await?;
    let mut responses = Vec::new();
    for o in orders {
        let items = state
            .order_repo
            .find_items_by_order_id(o.id, auth.tenant_id())
            .await?;
        responses.push(to_order_response(o, items));
    }
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
    let (o, items) = order::get_order(state.order_repo.as_ref(), &auth, &id).await?;
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
    order::ship_order(&state.pool, state.order_repo.as_ref(), &auth, &id, &req).await?;
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
    order::admin_cancel(&state.pool, state.order_repo.as_ref(), &auth, &id).await?;
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
    order::mark_paid(&state.pool, state.order_repo.as_ref(), &auth, &id).await?;
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
    order::refund_order(&state.pool, state.order_repo.as_ref(), &auth, &id).await?;
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
    order::update_admin_remark(state.order_repo.as_ref(), &auth, &id, &req.admin_remark).await?;
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
    let stats = order::get_stats(&state.pool, &auth).await?;
    Ok(ApiResponse::success(stats))
}

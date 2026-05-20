use axum::Json;
use axum::extract::{Path, State};

use crate::dto::cart::{AddToCartRequest, CartResponse, UpdateCartItemRequest};
use crate::dto::order::{OrderItemResponse, OrderResponse};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;

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
        "/cart",
        create,
        add_to_cart,
        "system",
        "cart"
    );
    let r = reg_route!(
        r, registry, restful, "/cart", get, list_cart, "system", "cart"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/cart/{id}",
        put,
        update_cart_item,
        "system",
        "cart"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/cart/{id}",
        delete,
        remove_from_cart,
        "system",
        "cart"
    );
    let r = reg_route!(
        r, registry, restful, "/cart", delete, clear_cart, "system", "cart"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/cart/checkout",
        post,
        checkout,
        "system",
        "cart"
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

#[utoipa::path(post, path = "/cart", tag = "cart",
    security(("bearer_auth" = [])),
    request_body = AddToCartRequest,
    responses((status = 200, description = "Item added to cart"))
)]
pub async fn add_to_cart(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<AddToCartRequest>,
) -> AppResult<ApiResponse<()>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    validation::validate(&req)?;
    state
        .cart_service
        .add_item(
            &auth,
            SnowflakeId(user_int_id),
            req.product_id,
            req.quantity,
            req.attributes,
        )
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(get, path = "/cart", tag = "cart",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Cart items list"))
)]
pub async fn list_cart(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<CartResponse>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    let cart = state
        .cart_service
        .list_items(&auth, SnowflakeId(user_int_id))
        .await?;
    Ok(ApiResponse::success(cart))
}

#[utoipa::path(put, path = "/cart/{id}", tag = "cart",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Cart item ID")),
    request_body = UpdateCartItemRequest,
    responses((status = 200, description = "Cart item updated"))
)]
pub async fn update_cart_item(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCartItemRequest>,
) -> AppResult<ApiResponse<()>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .cart_service
        .update_quantity(&auth, id, SnowflakeId(user_int_id), req.quantity)
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(delete, path = "/cart/{id}", tag = "cart",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Cart item ID")),
    responses((status = 200, description = "Cart item removed"))
)]
pub async fn remove_from_cart(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .cart_service
        .remove_item(&auth, id, SnowflakeId(user_int_id))
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(delete, path = "/cart", tag = "cart",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Cart cleared"))
)]
pub async fn clear_cart(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<()>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    state
        .cart_service
        .clear_cart(&auth, SnowflakeId(user_int_id))
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/cart/checkout", tag = "cart",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Order created from cart"))
)]
pub async fn checkout(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<OrderResponse>> {
    let _user_id = auth.ensure_authenticated()?;
    let user_int_id = auth
        .user_id()
        .ok_or(crate::errors::app_error::AppError::Unauthorized)?;
    let (order, items) = state
        .cart_service
        .checkout(&auth, SnowflakeId(user_int_id))
        .await?;
    Ok(ApiResponse::success(to_order_response(order, items)))
}

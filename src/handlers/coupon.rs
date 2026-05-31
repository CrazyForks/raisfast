use axum::Json;
use axum::extract::{Path, State};

use crate::dto::coupon::{CouponResponse, CreateCouponRequest, UpdateCouponRequest};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
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
        "/admin/coupons",
        get,
        admin_list,
        "system admin",
        "admin/coupons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/coupons",
        create,
        admin_create,
        "system admin",
        "admin/coupons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/coupons/{id}",
        put,
        admin_update,
        "system admin",
        "admin/coupons"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/coupons/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/coupons"
    )
}

#[utoipa::path(get, path = "/admin/coupons", tag = "coupons",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Coupon list"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<CouponResponse>>> {
    let mut p = params;
    p.sanitize();
    let (items, total) = state
        .coupon_service
        .list(&auth, p.page, p.page_size, None)
        .await?;
    Ok(p.paginate(items, total))
}

#[utoipa::path(post, path = "/admin/coupons", tag = "coupons",
    security(("bearer_auth" = [])),
    request_body = CreateCouponRequest,
    responses((status = 200, description = "Coupon created"))
)]
pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateCouponRequest>,
) -> AppResult<ApiResponse<CouponResponse>> {
    validation::validate(&req)?;
    let coupon = state.coupon_service.create(&auth, req).await?;
    Ok(ApiResponse::success(CouponResponse::from(coupon)))
}

#[utoipa::path(put, path = "/admin/coupons/{id}", tag = "coupons",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Coupon ID")),
    request_body = UpdateCouponRequest,
    responses((status = 200, description = "Coupon updated"))
)]
pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCouponRequest>,
) -> AppResult<ApiResponse<CouponResponse>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let coupon = state.coupon_service.update(&auth, id, req).await?;
    Ok(ApiResponse::success(CouponResponse::from(coupon)))
}

#[utoipa::path(delete, path = "/admin/coupons/{id}", tag = "coupons",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Coupon ID")),
    responses((status = 200, description = "Coupon deleted"))
)]
pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.coupon_service.delete(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

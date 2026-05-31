use axum::Json;
use axum::extract::{Path, State};

use crate::dto::shipping_template::{
    CalculateShippingRequest, CalculateShippingResponse, CreateShippingTemplateRequest,
    ShippingTemplateResponse, UpdateShippingTemplateRequest,
};
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
        "/admin/shipping-templates",
        get,
        admin_list,
        "system admin",
        "admin/shipping-templates"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/shipping-templates",
        create,
        admin_create,
        "system admin",
        "admin/shipping-templates"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/shipping-templates/{id}",
        put,
        admin_update,
        "system admin",
        "admin/shipping-templates"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/shipping-templates/calculate",
        post,
        calculate_shipping,
        "system authed",
        "shipping/calculate"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/shipping-templates/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/shipping-templates"
    )
}

#[utoipa::path(get, path = "/admin/shipping-templates", tag = "shipping-templates",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Shipping template list"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<ShippingTemplateResponse>>> {
    let mut p = params;
    p.sanitize();
    let (items, total) = state
        .shipping_template_service
        .list(&auth, p.page, p.page_size, None)
        .await?;
    Ok(p.paginate(items, total))
}

#[utoipa::path(post, path = "/admin/shipping-templates", tag = "shipping-templates",
    security(("bearer_auth" = [])),
    request_body = CreateShippingTemplateRequest,
    responses((status = 200, description = "Shipping template created"))
)]
pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateShippingTemplateRequest>,
) -> AppResult<ApiResponse<ShippingTemplateResponse>> {
    validation::validate(&req)?;
    let tmpl = state.shipping_template_service.create(&auth, req).await?;
    Ok(ApiResponse::success(ShippingTemplateResponse::from(tmpl)))
}

#[utoipa::path(put, path = "/admin/shipping-templates/{id}", tag = "shipping-templates",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Shipping template ID")),
    request_body = UpdateShippingTemplateRequest,
    responses((status = 200, description = "Shipping template updated"))
)]
pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateShippingTemplateRequest>,
) -> AppResult<ApiResponse<ShippingTemplateResponse>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let tmpl = state
        .shipping_template_service
        .update(&auth, id, req)
        .await?;
    Ok(ApiResponse::success(ShippingTemplateResponse::from(tmpl)))
}

#[utoipa::path(delete, path = "/admin/shipping-templates/{id}", tag = "shipping-templates",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Shipping template ID")),
    responses((status = 200, description = "Shipping template deleted"))
)]
pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.shipping_template_service.delete(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/shipping-templates/calculate", tag = "shipping-templates",
    security(("bearer_auth" = [])),
    request_body = CalculateShippingRequest,
    responses((status = 200, description = "Shipping calculated"))
)]
pub async fn calculate_shipping(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CalculateShippingRequest>,
) -> AppResult<ApiResponse<CalculateShippingResponse>> {
    let mut product_weights = Vec::new();
    for item in &req.items {
        let product_id = crate::types::snowflake_id::parse_id(&item.product_id)?;
        product_weights.push((product_id, 0, item.quantity));
    }
    let result = state
        .shipping_template_service
        .calculate_shipping(&product_weights, req.region.as_deref(), auth.tenant_id())
        .await?;
    Ok(ApiResponse::success(result))
}

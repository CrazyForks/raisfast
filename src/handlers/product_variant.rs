use crate::dto::product_variant::{
    CreateProductVariantRequest, ProductVariantResponse, UpdateProductVariantRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use axum::Json;
use axum::extract::{Path, State};

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
        "/products/{product_id}/variants",
        get,
        list_by_product,
        "system public",
        "product_variants"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product-variants",
        create,
        admin_create,
        "system admin",
        "admin/product_variants"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product-variants/{id}",
        put,
        admin_update,
        "system admin",
        "admin/product_variants"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/product-variants/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/product_variants"
    )
}

pub async fn list_by_product(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(product_id): Path<String>,
) -> AppResult<ApiResponse<Vec<ProductVariantResponse>>> {
    let variants = state
        .product_variant_service
        .list_by_product(&auth, &product_id)
        .await?;
    let resp: Vec<ProductVariantResponse> = variants.into_iter().map(Into::into).collect();
    Ok(ApiResponse::success(resp))
}

pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateProductVariantRequest>,
) -> AppResult<ApiResponse<ProductVariantResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let v = state.product_variant_service.create(&auth, req).await?;
    Ok(ApiResponse::success(ProductVariantResponse::from(v)))
}

pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProductVariantRequest>,
) -> AppResult<ApiResponse<ProductVariantResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let v = state.product_variant_service.update(&auth, id, req).await?;
    Ok(ApiResponse::success(ProductVariantResponse::from(v)))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.product_variant_service.delete(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

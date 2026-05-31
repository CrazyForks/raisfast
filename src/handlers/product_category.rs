use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{
    BatchRequest, BatchResponse, CreateProductCategoryRequest, ProductCategoryResponse,
    UpdateProductCategoryRequest,
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
        "/product-categories",
        get,
        self::list,
        "system public",
        "product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/product-categories",
        create,
        self::create,
        "system public",
        "product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/product-categories/{id}",
        get,
        self::get,
        "system public",
        "product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/product-categories/{id}",
        put,
        update,
        "system public",
        "product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/product-categories/{id}",
        delete,
        self::delete,
        "system public",
        "product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product-categories",
        get,
        admin_list,
        "system admin",
        "admin/product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product-categories",
        create,
        admin_create,
        "system admin",
        "admin/product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product-categories/{id}",
        put,
        admin_update,
        "system admin",
        "admin/product-categories"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product-categories/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/product-categories"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/product-categories/batch",
        post,
        admin_batch,
        "system admin",
        "admin/product-categories"
    )
}

pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<ProductCategoryResponse>>> {
    params.sanitize();
    let (items, total) = state
        .product_category_service
        .list_paginated(&auth, params.page, params.page_size)
        .await?;
    let items: Vec<ProductCategoryResponse> = items
        .into_iter()
        .map(ProductCategoryResponse::from_category)
        .collect();
    Ok(params.paginate(items, total))
}

pub async fn get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ProductCategoryResponse>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let cat = state.product_category_service.get(id, &auth).await?;
    Ok(ApiResponse::success(
        ProductCategoryResponse::from_category(cat),
    ))
}

pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateProductCategoryRequest>,
) -> AppResult<ApiResponse<ProductCategoryResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let cat = state.product_category_service.create(&auth, req).await?;
    Ok(ApiResponse::success(
        ProductCategoryResponse::from_category(cat),
    ))
}

pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProductCategoryRequest>,
) -> AppResult<ApiResponse<ProductCategoryResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let cat = state
        .product_category_service
        .update(&auth, id, req)
        .await?;
    Ok(ApiResponse::success(
        ProductCategoryResponse::from_category(cat),
    ))
}

pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.product_category_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<ProductCategoryResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = state
        .product_category_service
        .list_paginated(&auth, params.page, params.page_size)
        .await?;
    let items: Vec<ProductCategoryResponse> = items
        .into_iter()
        .map(ProductCategoryResponse::from_category)
        .collect();
    Ok(params.paginate(items, total))
}

pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateProductCategoryRequest>,
) -> AppResult<ApiResponse<ProductCategoryResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let cat = state.product_category_service.create(&auth, req).await?;
    Ok(ApiResponse::success(
        ProductCategoryResponse::from_category(cat),
    ))
}

pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProductCategoryRequest>,
) -> AppResult<ApiResponse<ProductCategoryResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let cat = state
        .product_category_service
        .update(&auth, id, req)
        .await?;
    Ok(ApiResponse::success(
        ProductCategoryResponse::from_category(cat),
    ))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.product_category_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let mut affected = 0usize;
    if req.action == "delete" {
        for raw_id in &req.ids {
            if let Ok(id) = crate::types::snowflake_id::parse_id(raw_id)
                && state
                    .product_category_service
                    .delete(id, &auth)
                    .await
                    .is_ok()
            {
                affected += 1;
            }
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}

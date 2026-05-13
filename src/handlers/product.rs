use axum::Json;
use axum::extract::{Path, Query, State};
use axum::routing::{get, put};

use crate::dto::{
    CreateProductRequest, UpdateProductRequest, ProductResponse,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::product;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    let r = axum::Router::new();
    let r = crate::reg_route!(
        r,
        registry,
        "/products",
        get(list_active),
        "system public",
        "products",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/products/{id}",
        get(get_product),
        "system public",
        "products",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/products",
        get(admin_list).post(admin_create),
        "system admin",
        "admin/products",
        ["GET", "POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/products/{id}",
        put(admin_update).delete(admin_delete),
        "system admin",
        "admin/products",
        ["PUT", "DELETE"]
    );
    #[allow(clippy::let_and_return)]
    r
}

pub async fn list_active(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<ProductResponse>>> {
    params.sanitize();
    let (items, total) =
        product::list_active_products(state.product_repo.as_ref(), &auth, params.page, params.page_size)
            .await?;
    let resp: Vec<ProductResponse> = items.into_iter().map(Into::into).collect();
    Ok(params.paginate(resp, total))
}

pub async fn get_product(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ProductResponse>> {
    let p = product::get_product(state.product_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(ProductResponse::from(p)))
}

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<ProductResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) =
        product::list_admin_products(state.product_repo.as_ref(), &auth, params.page, params.page_size, None)
            .await?;
    let resp: Vec<ProductResponse> = items.into_iter().map(Into::into).collect();
    Ok(params.paginate(resp, total))
}

pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateProductRequest>,
) -> AppResult<ApiResponse<ProductResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let p = product::create_product(state.product_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(ProductResponse::from(p)))
}

pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProductRequest>,
) -> AppResult<ApiResponse<ProductResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let p = product::update_product(state.product_repo.as_ref(), &auth, &id, req).await?;
    Ok(ApiResponse::success(ProductResponse::from(p)))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    product::delete_product(state.product_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

use crate::dto::{CreateProductRequest, ProductResponse, UpdateProductRequest};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::product;
use crate::utils::pagination::PaginationParams;
use axum::Json;
use axum::extract::{Path, Query, State};

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
        "/products",
        get,
        list_active,
        "system public",
        "products"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/products/{id}",
        get,
        get_product,
        "system public",
        "products"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/products",
        get,
        admin_list,
        "system admin",
        "admin/products"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/products",
        create,
        admin_create,
        "system admin",
        "admin/products"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/products/{id}",
        put,
        admin_update,
        "system admin",
        "admin/products"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/products/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/products"
    )
}

#[utoipa::path(get, path = "/products", tag = "products",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List active products"))
)]
pub async fn list_active(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<ProductResponse>>> {
    params.sanitize();
    let (items, total) = product::list_active_products(
        state.product_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
    )
    .await?;
    let resp: Vec<ProductResponse> = items.into_iter().map(Into::into).collect();
    Ok(params.paginate(resp, total))
}

#[utoipa::path(get, path = "/products/{id}", tag = "products",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Product ID")),
    responses((status = 200, description = "Product detail"))
)]
pub async fn get_product(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ProductResponse>> {
    let p = product::get_product(state.product_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(ProductResponse::from(p)))
}

#[utoipa::path(get, path = "/admin/products", tag = "products",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin product list"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<ProductResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = product::list_admin_products(
        state.product_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
        None,
    )
    .await?;
    let resp: Vec<ProductResponse> = items.into_iter().map(Into::into).collect();
    Ok(params.paginate(resp, total))
}

#[utoipa::path(post, path = "/admin/products", tag = "products",
    security(("bearer_auth" = [])),
    request_body = CreateProductRequest,
    responses((status = 200, description = "Product created"))
)]
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

#[utoipa::path(put, path = "/admin/products/{id}", tag = "products",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Product ID")),
    request_body = UpdateProductRequest,
    responses((status = 200, description = "Product updated"))
)]
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

#[utoipa::path(delete, path = "/admin/products/{id}", tag = "products",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Product ID")),
    responses((status = 200, description = "Product deleted"))
)]
pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    product::delete_product(state.product_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

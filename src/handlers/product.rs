use crate::dto::{
    BatchRequest, BatchResponse, CreateProductRequest, ProductResponse, UpdateProductRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
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
        "/products/{slug}",
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
        "/admin/products/batch",
        post,
        admin_batch,
        "system admin",
        "admin/products"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/products/{id}",
        get,
        admin_get,
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
    let (items, total) = state
        .product_service
        .list_active(&auth, params.page, params.page_size)
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
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<ProductResponse>> {
    let p = crate::models::product::find_by_slug(&state.pool, &slug, auth.tenant_id())
        .await?
        .ok_or_else(|| crate::errors::app_error::AppError::not_found("product"))?;
    let mut resp = ProductResponse::from(p);
    let id = crate::types::snowflake_id::parse_id(&resp.id)?;
    resp.tags = crate::models::tagging::get_tags_for(&state.pool, "product", id).await?;
    Ok(ApiResponse::success(resp))
}

#[utoipa::path(get, path = "/admin/products", tag = "products",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin product list"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<crate::dto::product::AdminProductListQuery>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<ProductResponse>>> {
    auth.ensure_admin()?;
    let params = PaginationParams::from_options(query.page, query.page_size);
    let (items, total) = state
        .product_service
        .list_admin(
            &auth,
            params.page,
            params.page_size,
            query.status.as_deref(),
            query.keyword.as_deref(),
            query.category_id.as_deref(),
        )
        .await?;
    let resp: Vec<ProductResponse> = items.into_iter().map(Into::into).collect();
    Ok(params.paginate(resp, total))
}

#[utoipa::path(get, path = "/admin/products/{id}", tag = "products",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Product ID")),
    responses((status = 200, description = "Admin product detail"))
)]
pub async fn admin_get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ProductResponse>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let p = state.product_service.get(id, &auth).await?;
    let mut resp = ProductResponse::from(p);
    resp.tags = crate::models::tagging::get_tags_for(&state.pool, "product", id).await?;
    Ok(ApiResponse::success(resp))
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
    let p = state.product_service.create(&auth, req).await?;
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let p = state.product_service.update(&auth, id, req).await?;
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.product_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let affected = state
        .product_service
        .batch(&auth, &req.action, &req.ids)
        .await?;
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}

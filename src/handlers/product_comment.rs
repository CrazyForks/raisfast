use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::product_comment::{
    AdminProductCommentListQuery, AdminReplyRequest, CreateProductCommentRequest,
    ProductCommentResponse, UpdateProductCommentRequest, UpdateProductCommentStatusRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::product_comment::ProductCommentStats;
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
        "/products/{product_id}/comments",
        get,
        list_by_product,
        "system public",
        "product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/products/{product_id}/comments/stats",
        get,
        get_stats,
        "system public",
        "product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/product_comments",
        post,
        create,
        "system authed",
        "product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/product_comments/{id}",
        put,
        update,
        "system authed",
        "product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/product_comments/{id}",
        delete,
        delete,
        "system authed",
        "product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/users/{user_id}/product_comments",
        get,
        list_by_user,
        "system authed",
        "product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product_comments",
        get,
        admin_list,
        "system admin",
        "admin/product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product_comments/{id}/status",
        put,
        admin_update_status,
        "system admin",
        "admin/product_comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/product_comments/{id}/reply",
        put,
        admin_reply,
        "system admin",
        "admin/product_comments"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/product_comments/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/product_comments"
    )
}

#[utoipa::path(get, path = "/products/{product_id}/comments", tag = "product_comments",
    params(("product_id" = String, Path, description = "Product ID")),
    responses((status = 200, description = "Product comment list"))
)]
pub async fn list_by_product(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(product_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<ProductCommentResponse>>> {
    let mut p = params;
    p.sanitize();
    let (items, total) = state
        .product_comment_service
        .list_by_product(&auth, &product_id, p.page, p.page_size)
        .await?;
    Ok(p.paginate(items, total))
}

#[utoipa::path(get, path = "/products/{product_id}/comments/stats", tag = "product_comments",
    params(("product_id" = String, Path, description = "Product ID")),
    responses((status = 200, description = "Product comment stats"))
)]
pub async fn get_stats(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(product_id): Path<String>,
) -> AppResult<ApiResponse<ProductCommentStats>> {
    let stats = state
        .product_comment_service
        .get_stats(&auth, &product_id)
        .await?;
    Ok(ApiResponse::success(stats))
}

#[utoipa::path(post, path = "/product_comments", tag = "product_comments",
    security(("bearer_auth" = [])),
    request_body = CreateProductCommentRequest,
    responses((status = 200, description = "Product comment created"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateProductCommentRequest>,
) -> AppResult<ApiResponse<ProductCommentResponse>> {
    auth.ensure_authenticated()?;
    validation::validate(&req)?;
    let comment = state.product_comment_service.create(&auth, req).await?;
    Ok(ApiResponse::success(ProductCommentResponse::from(comment)))
}

#[utoipa::path(put, path = "/product_comments/{id}", tag = "product_comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    request_body = UpdateProductCommentRequest,
    responses((status = 200, description = "Product comment updated"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProductCommentRequest>,
) -> AppResult<ApiResponse<ProductCommentResponse>> {
    auth.ensure_authenticated()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let comment = state.product_comment_service.update(&auth, id, req).await?;
    Ok(ApiResponse::success(ProductCommentResponse::from(comment)))
}

#[utoipa::path(delete, path = "/product_comments/{id}", tag = "product_comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    responses((status = 200, description = "Product comment deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_authenticated()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.product_comment_service.delete(&auth, id).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(get, path = "/users/{user_id}/product_comments", tag = "product_comments",
    security(("bearer_auth" = [])),
    params(("user_id" = String, Path, description = "User ID")),
    responses((status = 200, description = "User's product comments"))
)]
pub async fn list_by_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(user_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<ProductCommentResponse>>> {
    auth.ensure_authenticated()?;
    let mut p = params;
    p.sanitize();
    let uid = crate::types::snowflake_id::parse_id(&user_id)?;
    let (items, total) = state
        .product_comment_service
        .list_by_user(&auth, uid, p.page, p.page_size)
        .await?;
    Ok(p.paginate(items, total))
}

#[utoipa::path(get, path = "/admin/product_comments", tag = "product_comments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin product comment list"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AdminProductCommentListQuery>,
) -> AppResult<ApiResponse<PaginatedData<ProductCommentResponse>>> {
    auth.ensure_admin()?;
    let (items, total) = state
        .product_comment_service
        .admin_list(&auth, &query)
        .await?;
    let pagination = PaginationParams::from_options(query.page, query.page_size);
    Ok(pagination.paginate(
        items
            .into_iter()
            .map(ProductCommentResponse::from)
            .collect(),
        total,
    ))
}

#[utoipa::path(put, path = "/admin/product_comments/{id}/status", tag = "product_comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    request_body = UpdateProductCommentStatusRequest,
    responses((status = 200, description = "Comment status updated"))
)]
pub async fn admin_update_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProductCommentStatusRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .product_comment_service
        .admin_update_status(&auth, id, req.status)
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(put, path = "/admin/product_comments/{id}/reply", tag = "product_comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    request_body = AdminReplyRequest,
    responses((status = 200, description = "Admin reply added"))
)]
pub async fn admin_reply(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<AdminReplyRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .product_comment_service
        .admin_reply(&auth, id, &req.admin_reply)
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(delete, path = "/admin/product_comments/{id}", tag = "product_comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    responses((status = 200, description = "Comment deleted"))
)]
pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .product_comment_service
        .admin_delete(&auth, id)
        .await?;
    Ok(ApiResponse::success(()))
}

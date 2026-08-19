use crate::types::price::Price;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProductFavoriteItem {
    /// Product id (use slug for links when present).
    pub product_id: SnowflakeId,
    pub favorited_at: String,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub cover_image: Option<String>,
    pub price: Option<Price>,
    pub original_price: Option<Price>,
    pub status: Option<String>,
    pub stock: Option<i64>,
    pub sales: Option<i64>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FavoriteStatusResponse {
    pub product_id: SnowflakeId,
    pub favorited: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ToggleFavoriteRequest {
    pub product_id: SnowflakeId,
}

#[allow(clippy::let_and_return)]
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
        "/favorites",
        get,
        list_favorites,
        "ecommerce",
        "favorites",
        "product_favorites:read"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/favorites/{product_id}",
        get,
        favorite_status,
        "ecommerce",
        "favorites",
        "product_favorites:read"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/favorites/{product_id}",
        delete,
        remove_favorite,
        "ecommerce",
        "favorites",
        "product_favorites:delete"
    );
    let router = reg_route!(
        r,
        registry,
        restful,
        "/favorites/{product_id}",
        post,
        add_favorite,
        "ecommerce",
        "favorites",
        "product_favorites:create"
    );
    router
}

/// List the current user's favorite products (newest first, paginated,
/// joined with a product snapshot so no N+1 lookups are needed).
#[utoipa::path(get, path = "/favorites", tag = "favorites",
    security(("bearer_auth" = [])),
    params(
        ("page" = Option<i64>, Query, description = "Page number (1-based)"),
        ("page_size" = Option<i64>, Query, description = "Items per page")
    ),
    responses((status = 200, description = "Favorite list"))
)]
pub async fn list_favorites(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    axum::extract::Query(params): axum::extract::Query<crate::utils::pagination::PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<ProductFavoriteItem>>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let mut params = params;
    params.sanitize();
    let (rows, total) = crate::models::product_favorite::find_paged_with_product(
        &state.pool,
        user_id,
        params.page,
        params.page_size,
        auth.tenant_id(),
    )
    .await?;
    let items = rows
        .into_iter()
        .map(|f| ProductFavoriteItem {
            product_id: f.product_id,
            favorited_at: f.created_at.to_string(),
            title: f.product_title,
            slug: f.product_slug,
            cover_image: f.product_cover_image,
            price: f.product_price,
            original_price: f.product_original_price,
            status: f.product_status,
            stock: f.product_stock,
            sales: f.product_sales,
        })
        .collect();
    Ok(params.paginate(items, total))
}

/// Check whether the current user has favorited a product.
#[utoipa::path(get, path = "/favorites/{product_id}", tag = "favorites",
    security(("bearer_auth" = [])),
    params(("product_id" = String, Path, description = "Product id")),
    responses((status = 200, description = "Favorite status"))
)]
pub async fn favorite_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(product_id): Path<String>,
) -> AppResult<ApiResponse<FavoriteStatusResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let pid = crate::types::snowflake_id::parse_id(&product_id)
        .map_err(|_| AppError::BadRequest("invalid_product_id".into()))?;
    let existing = crate::models::product_favorite::find_by_user_and_product(
        &state.pool,
        user_id,
        pid,
        auth.tenant_id(),
    )
    .await?;
    Ok(ApiResponse::success(FavoriteStatusResponse {
        product_id: pid,
        favorited: existing.is_some(),
    }))
}

/// Add a product to favorites (idempotent).
#[utoipa::path(post, path = "/favorites/{product_id}", tag = "favorites",
    security(("bearer_auth" = [])),
    params(("product_id" = String, Path, description = "Product id")),
    responses((status = 200, description = "Added"))
)]
pub async fn add_favorite(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(product_id): Path<String>,
) -> AppResult<ApiResponse<FavoriteStatusResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let pid = crate::types::snowflake_id::parse_id(&product_id)
        .map_err(|_| AppError::BadRequest("invalid_product_id".into()))?;
    let existing = crate::models::product_favorite::find_by_user_and_product(
        &state.pool,
        user_id,
        pid,
        auth.tenant_id(),
    )
    .await?;
    if existing.is_none() {
        crate::in_transaction!(&state.pool, tx, {
            crate::models::product_favorite::tx_create(&mut tx, user_id, pid, auth.tenant_id())
                .await
        })?;
    }
    Ok(ApiResponse::success(FavoriteStatusResponse {
        product_id: pid,
        favorited: true,
    }))
}

/// Remove a product from favorites (idempotent).
#[utoipa::path(delete, path = "/favorites/{product_id}", tag = "favorites",
    security(("bearer_auth" = [])),
    params(("product_id" = String, Path, description = "Product id")),
    responses((status = 200, description = "Removed"))
)]
pub async fn remove_favorite(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(product_id): Path<String>,
) -> AppResult<ApiResponse<FavoriteStatusResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let pid = crate::types::snowflake_id::parse_id(&product_id)
        .map_err(|_| AppError::BadRequest("invalid_product_id".into()))?;
    crate::in_transaction!(&state.pool, tx, {
        crate::models::product_favorite::tx_delete_by_user_and_product(
            &mut tx,
            user_id,
            pid,
            auth.tenant_id(),
        )
        .await
    })?;
    Ok(ApiResponse::success(FavoriteStatusResponse {
        product_id: pid,
        favorited: false,
    }))
}

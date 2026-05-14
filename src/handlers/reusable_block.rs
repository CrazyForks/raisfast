//! Reusable block handler

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use validator::Validate;

use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::reusable_block as reusable_service;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/admin/reusable-blocks",
        get(list_reusable).post(create_reusable),
        "system admin",
        "admin/pages",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/reusable-blocks/{id}",
        get(get_reusable)
            .put(update_reusable)
            .delete(delete_reusable),
        "system admin",
        "admin/pages",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/reusable-blocks/batch",
        http_post(admin_batch),
        "system admin",
        "admin/pages",
        ["POST"]
    )
}

#[derive(Debug, Deserialize, Validate, utoipa::ToSchema)]
pub struct CreateReusableRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    #[validate(length(min = 1))]
    pub block_type: String,
    #[validate(length(min = 1))]
    pub content: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Validate, utoipa::ToSchema)]
pub struct UpdateReusableRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: Option<String>,
    #[validate(length(min = 1))]
    pub block_type: Option<String>,
    #[validate(length(min = 1))]
    pub content: Option<String>,
    pub description: Option<String>,
}

#[utoipa::path(get, path = "/admin/reusable-blocks", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Reusable block list"))
)]
pub async fn list_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<crate::models::reusable_block::ReusableBlock>>> {
    auth.ensure_author()?;
    let items = reusable_service::list_reusable(&state.pool, &auth).await?;
    Ok(ApiResponse::success(items))
}

#[utoipa::path(get, path = "/admin/reusable-blocks/{id}", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Block ID")),
    responses((status = 200, description = "Reusable block details"))
)]
pub async fn get_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<crate::models::reusable_block::ReusableBlock>> {
    auth.ensure_author()?;
    let block = reusable_service::get_reusable(&state.pool, &id, &auth)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    Ok(ApiResponse::success(block))
}

#[utoipa::path(post, path = "/admin/reusable-blocks", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Reusable block created"))
)]
pub async fn create_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateReusableRequest>,
) -> AppResult<ApiResponse<crate::models::reusable_block::ReusableBlock>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let block = reusable_service::create_reusable(
        &state.pool,
        &auth,
        &req.name,
        &req.block_type,
        &req.content,
        req.description.as_deref(),
    )
    .await?;
    Ok(ApiResponse::success(block))
}

#[utoipa::path(put, path = "/admin/reusable-blocks/{id}", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Block ID")),
    responses((status = 200, description = "Reusable block updated"))
)]
pub async fn update_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateReusableRequest>,
) -> AppResult<ApiResponse<crate::models::reusable_block::ReusableBlock>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let block = reusable_service::update_reusable(
        &state.pool,
        &id,
        &auth,
        req.name.as_deref(),
        req.block_type.as_deref(),
        req.content.as_deref(),
        req.description.as_deref(),
    )
    .await?;
    Ok(ApiResponse::success(block))
}

#[utoipa::path(delete, path = "/admin/reusable-blocks/{id}", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Block ID")),
    responses((status = 200, description = "Reusable block deleted"))
)]
pub async fn delete_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    reusable_service::delete_reusable(&state.pool, &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/reusable-blocks/batch", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    request_body = BatchRequest,
    responses((status = 200, description = "Batch operation completed"))
)]
pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let mut affected = 0usize;
    if req.action == "delete" {
        for id in &req.ids {
            if reusable_service::delete_reusable(&state.pool, id, &auth)
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

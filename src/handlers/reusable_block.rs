//! Reusable block handler

use axum::Json;
use axum::extract::{Path, State};

use crate::dto::{
    BatchRequest, BatchResponse, CreateReusableRequest, ReusableBlockResponse,
    UpdateReusableRequest,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::reusable_block as reusable_service;

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
        "/admin/reusable-blocks",
        get,
        list_reusable,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/reusable-blocks",
        create,
        create_reusable,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/reusable-blocks/{id}",
        get,
        get_reusable,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/reusable-blocks/{id}",
        put,
        update_reusable,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/reusable-blocks/{id}",
        delete,
        delete_reusable,
        "system admin",
        "admin/pages"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/reusable-blocks/batch",
        post,
        admin_batch,
        "system admin",
        "admin/pages"
    )
}

#[utoipa::path(get, path = "/admin/reusable-blocks", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Reusable block list"))
)]
pub async fn list_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<ReusableBlockResponse>>> {
    auth.ensure_author()?;
    let items = reusable_service::list_reusable(&state.pool, &auth).await?;
    let items: Vec<ReusableBlockResponse> = items
        .into_iter()
        .map(ReusableBlockResponse::from_block)
        .collect();
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
) -> AppResult<ApiResponse<ReusableBlockResponse>> {
    auth.ensure_author()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let block = reusable_service::get_reusable(&state.pool, id, &auth)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    Ok(ApiResponse::success(ReusableBlockResponse::from_block(
        block,
    )))
}

#[utoipa::path(post, path = "/admin/reusable-blocks", tag = "reusable_blocks",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Reusable block created"))
)]
pub async fn create_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateReusableRequest>,
) -> AppResult<ApiResponse<ReusableBlockResponse>> {
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
    Ok(ApiResponse::success(ReusableBlockResponse::from_block(
        block,
    )))
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
) -> AppResult<ApiResponse<ReusableBlockResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let block = reusable_service::update_reusable(
        &state.pool,
        id,
        &auth,
        req.name.as_deref(),
        req.block_type.as_deref(),
        req.content.as_deref(),
        req.description.as_deref(),
    )
    .await?;
    Ok(ApiResponse::success(ReusableBlockResponse::from_block(
        block,
    )))
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    reusable_service::delete_reusable(&state.pool, id, &auth).await?;
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
        for raw_id in &req.ids {
            if let Ok(id) = crate::types::snowflake_id::parse_id(raw_id)
                && reusable_service::delete_reusable(&state.pool, id, &auth)
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

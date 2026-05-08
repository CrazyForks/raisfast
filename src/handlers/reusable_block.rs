//! 可复用块处理器

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use validator::Validate;

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::reusable_block as reusable_service;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateReusableRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    #[validate(length(min = 1))]
    pub block_type: String,
    #[validate(length(min = 1))]
    pub content: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateReusableRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: Option<String>,
    #[validate(length(min = 1))]
    pub block_type: Option<String>,
    #[validate(length(min = 1))]
    pub content: Option<String>,
    pub description: Option<String>,
}

pub async fn list_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<crate::models::reusable_block::ReusableBlock>>> {
    auth.ensure_author()?;
    let items = reusable_service::list_reusable(&state.pool, &auth).await?;
    Ok(ApiResponse::success(items))
}

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

pub async fn delete_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    reusable_service::delete_reusable(&state.pool, &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

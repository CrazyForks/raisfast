//! 页面与块处理器
//!
//! 处理页面 CRUD、状态变更、排序、站点地图，以及可复用块管理。

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use validator::Validate;

use crate::commands::{CreatePageCmd, UpdatePageCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::page as page_service;
use crate::utils::pagination::PaginationParams;

// ── DTO ──

#[derive(Debug, Deserialize, Validate)]
pub struct CreatePageRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
    pub status: Option<String>,
    pub cover_image: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdatePageRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: Option<String>,
    pub parent_id: Option<Option<String>>,
    pub sort_order: Option<i64>,
    pub status: Option<String>,
    pub cover_image: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PageListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AdminPageListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct ReorderRequest {
    pub items: Vec<ReorderItem>,
}

#[derive(Debug, Deserialize)]
pub struct ReorderItem {
    pub id: String,
    pub sort_order: i64,
}

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

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct SitemapEntry {
    pub slug: String,
    pub updated_at: Option<String>,
}

// ── 公开 API ──

pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<PageListQuery>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::page::Page>>> {
    let mut pagination = PaginationParams::default();
    if let Some(page) = query.page {
        pagination.page = page.max(1);
    }
    if let Some(page_size) = query.page_size {
        pagination.page_size = page_size.clamp(1, 100);
    }
    pagination.sanitize();

    let (items, total) =
        page_service::list_published(&state.pool, pagination.page, pagination.page_size, &auth)
            .await?;

    Ok(pagination.paginate(items, total))
}

pub async fn get_by_slug(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<crate::models::page::Page>> {
    let page = page_service::get_by_slug(&state.pool, &slug, &auth).await?;
    Ok(ApiResponse::success(page))
}

pub async fn sitemap(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<SitemapEntry>>> {
    let entries = page_service::sitemap(&state.pool, &auth)
        .await?
        .into_iter()
        .map(|(slug, updated_at)| SitemapEntry { slug, updated_at })
        .collect();
    Ok(ApiResponse::success(entries))
}

// ── 管理 API ──

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AdminPageListQuery>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::page::Page>>> {
    auth.ensure_author()?;
    let mut pagination = PaginationParams::default();
    if let Some(page) = query.page {
        pagination.page = page.max(1);
    }
    if let Some(page_size) = query.page_size {
        pagination.page_size = page_size.clamp(1, 100);
    }
    pagination.sanitize();

    let (items, total) = page_service::list_all(
        &state.pool,
        pagination.page,
        pagination.page_size,
        query.status.as_deref(),
        &auth,
    )
    .await?;

    Ok(pagination.paginate(items, total))
}

pub async fn admin_get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<crate::models::page::Page>> {
    auth.ensure_author()?;
    let page = page_service::get_by_id(&state.pool, &id, &auth).await?;
    Ok(ApiResponse::success(page))
}

pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreatePageRequest>,
) -> AppResult<ApiResponse<crate::models::page::Page>> {
    auth.ensure_author()?;
    validation::validate(&req)?;

    let slug = req
        .slug
        .unwrap_or_else(|| page_service::generate_slug(&req.title));
    let template = req.template.unwrap_or_else(|| "default".to_string());
    let status = req.status.unwrap_or_else(|| "draft".to_string());

    let cmd = CreatePageCmd {
        title: req.title,
        slug,
        content: req.content,
        blocks: req.blocks,
        meta_title: req.meta_title,
        meta_description: req.meta_description,
        og_image: req.og_image,
        template,
        parent_id: req.parent_id,
        sort_order: req.sort_order.unwrap_or(0),
        status,
        created_by: auth.ensure_authenticated()?.to_string(),
        updated_by: None,
        cover_image: req.cover_image,
    };

    let page = page_service::create_page(&state.pool, &auth, cmd).await?;
    Ok(ApiResponse::success(page))
}

pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePageRequest>,
) -> AppResult<ApiResponse<crate::models::page::Page>> {
    auth.ensure_author()?;
    validation::validate(&req)?;

    let cmd = UpdatePageCmd {
        id,
        title: req.title,
        slug: req.slug,
        content: req.content,
        blocks: req.blocks,
        meta_title: req.meta_title,
        meta_description: req.meta_description,
        og_image: req.og_image,
        template: req.template,
        parent_id: req.parent_id,
        sort_order: req.sort_order,
        status: req.status,
        cover_image: req.cover_image,
        updated_by: None,
    };

    let page = page_service::update_page(&state.pool, &auth, cmd).await?;
    Ok(ApiResponse::success(page))
}

pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    page_service::delete_page(&state.pool, &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

pub async fn update_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> AppResult<ApiResponse<crate::models::page::Page>> {
    auth.ensure_author()?;
    let page = page_service::update_status(&state.pool, &id, &req.status, &auth).await?;
    Ok(ApiResponse::success(page))
}

pub async fn reorder(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<ReorderRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    let items: Vec<(String, i64)> = req
        .items
        .into_iter()
        .map(|i| (i.id, i.sort_order))
        .collect();
    page_service::reorder(&state.pool, items, &auth).await?;
    Ok(ApiResponse::success(()))
}

// ── 可复用块 ──

pub async fn list_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<crate::models::page::ReusableBlock>>> {
    auth.ensure_author()?;
    let items = page_service::list_reusable(&state.pool, &auth).await?;
    Ok(ApiResponse::success(items))
}

pub async fn get_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<crate::models::page::ReusableBlock>> {
    auth.ensure_author()?;
    let block = page_service::get_reusable(&state.pool, &id, &auth)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    Ok(ApiResponse::success(block))
}

pub async fn create_reusable(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateReusableRequest>,
) -> AppResult<ApiResponse<crate::models::page::ReusableBlock>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let block = page_service::create_reusable(
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
) -> AppResult<ApiResponse<crate::models::page::ReusableBlock>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let block = page_service::update_reusable(
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
    page_service::delete_reusable(&state.pool, &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

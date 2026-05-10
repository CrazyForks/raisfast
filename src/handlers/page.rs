//! 页面处理器
//!
//! 处理页面 CRUD、状态变更、排序与站点地图。

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use validator::Validate;

use crate::commands::{CreatePageCmd, UpdatePageCmd};
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::{page as page_service, post::resolve_doc_id_to_int};
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post, put};

    let r = axum::Router::new();
    let r = reg_route!(r, registry, "/pages", get(self::list).post(create), "system public", "pages", ["GET", "POST"]);
    let r = reg_route!(r, registry, "/pages/sitemap", get(sitemap), "system public", "pages", ["GET"]);
    let r = reg_route!(r, registry, "/pages/{slug}", get(get_by_slug), "system public", "pages", ["GET"]);
    let r = reg_route!(r, registry, "/admin/pages", get(admin_list).post(create), "system admin", "admin/pages", ["GET", "POST"]);
    let r = reg_route!(r, registry, "/admin/pages/{id}", get(admin_get).put(update).delete(self::delete), "system admin", "admin/pages", ["GET", "PUT", "DELETE"]);
    let r = reg_route!(r, registry, "/admin/pages/{id}/status", put(update_status), "system admin", "admin/pages", ["PUT"]);
    let r = reg_route!(r, registry, "/admin/pages/reorder", put(reorder), "system admin", "admin/pages", ["PUT"]);
    reg_route!(r, registry, "/admin/pages/batch", http_post(admin_batch), "system admin", "admin/pages", ["POST"])
}


async fn resolve_page_parent_id(
    pool: &crate::db::Pool,
    parent_id: Option<String>,
) -> AppResult<Option<i64>> {
    let Some(doc_id) = parent_id else {
        return Ok(None);
    };
    resolve_doc_id_to_int(pool, "pages", &doc_id, None).await
}

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
    let pagination = PaginationParams::from_options(query.page, query.page_size);

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
    let pagination = PaginationParams::from_options(query.page, query.page_size);

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

    let resolved_parent_id = resolve_page_parent_id(&state.pool, req.parent_id).await?;
    let cmd = CreatePageCmd {
        title: req.title,
        slug,
        content: req.content,
        blocks: req.blocks,
        meta_title: req.meta_title,
        meta_description: req.meta_description,
        og_image: req.og_image,
        template,
        parent_id: resolved_parent_id,
        sort_order: req.sort_order.unwrap_or(0),
        status,
        created_by: auth.user_int_id().ok_or(AppError::Unauthorized)?,
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

    let resolved_parent_id = resolve_page_parent_id(&state.pool, req.parent_id.flatten()).await?;
    let cmd = UpdatePageCmd {
        id: 0,
        title: req.title,
        slug: req.slug,
        content: req.content,
        blocks: req.blocks,
        meta_title: req.meta_title,
        meta_description: req.meta_description,
        og_image: req.og_image,
        template: req.template,
        parent_id: Some(resolved_parent_id),
        sort_order: req.sort_order,
        status: req.status,
        cover_image: req.cover_image,
        updated_by: auth.user_int_id(),
    };

    let page = page_service::update_page(&state.pool, &auth, &id, cmd).await?;
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

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let mut affected = 0usize;
    for id in &req.ids {
        match req.action.as_str() {
            "delete" => {
                if page_service::delete_page(&state.pool, id, &auth).await.is_ok() {
                    affected += 1;
                }
            }
            "publish" | "unpublish" => {
                let status = if req.action == "publish" {
                    "published"
                } else {
                    "draft"
                };
                if page_service::update_status(&state.pool, id, status, &auth)
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(&req.action, affected)))
}

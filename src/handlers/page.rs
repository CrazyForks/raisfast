//! Page handlers
//!
//! Handles page CRUD, status changes, ordering, and sitemap.

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::commands::{CreatePageCmd, UpdatePageCmd};
use crate::dto::{
    AdminPageListQuery, BatchRequest, BatchResponse, CreatePageRequest, PageListQuery,
    PageResponse, ReorderRequest, SitemapEntry, UpdatePageRequest, UpdateStatusRequest,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::page::PageStatus;
use crate::types::snowflake_id::SnowflakeId;
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
        "/pages",
        get,
        self::list,
        "system public",
        "pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/pages",
        create,
        self::create,
        "system public",
        "pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/pages/sitemap",
        get,
        sitemap,
        "system public",
        "pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/pages/{slug}",
        get,
        get_by_slug,
        "system public",
        "pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/pages",
        get,
        admin_list,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/pages",
        create,
        self::create,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/pages/{id}",
        get,
        admin_get,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/pages/{id}",
        put,
        update,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/pages/{id}",
        delete,
        self::delete,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/pages/{id}/status",
        put,
        update_status,
        "system admin",
        "admin/pages"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/pages/reorder",
        put,
        reorder,
        "system admin",
        "admin/pages"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/pages/batch",
        post,
        admin_batch,
        "system admin",
        "admin/pages"
    )
}

async fn resolve_page_parent_id(
    pool: &crate::db::Pool,
    parent_id: Option<String>,
) -> AppResult<Option<i64>> {
    let Some(raw_id) = parent_id else {
        return Ok(None);
    };
    let parsed_id = crate::types::snowflake_id::parse_id(&raw_id)?;
    raisfast_derive::crud_resolve_id!(pool, "pages", *parsed_id).map_err(Into::into)
}

// ── Public API ──

#[utoipa::path(get, path = "/pages", tag = "pages",
    responses((status = 200, description = "Published page list"))
)]
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<PageListQuery>,
) -> AppResult<ApiResponse<PaginatedData<PageResponse>>> {
    let pagination = PaginationParams::from_options(query.page, query.page_size);

    let (items, total) = state
        .page_service
        .list_published(pagination.page, pagination.page_size, &auth)
        .await?;

    let items: Vec<PageResponse> = items.into_iter().map(PageResponse::from_page).collect();
    Ok(pagination.paginate(items, total))
}

#[utoipa::path(get, path = "/pages/{slug}", tag = "pages",
    params(("slug" = String, Path, description = "Page slug")),
    responses((status = 200, description = "Page details"))
)]
pub async fn get_by_slug(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<PageResponse>> {
    let page = state.page_service.get_by_slug(&slug, &auth).await?;
    Ok(ApiResponse::success(PageResponse::from_page(page)))
}

#[utoipa::path(get, path = "/pages/sitemap", tag = "pages",
    responses((status = 200, description = "Sitemap entries"))
)]
pub async fn sitemap(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<SitemapEntry>>> {
    let entries = state
        .page_service
        .sitemap(&auth)
        .await?
        .into_iter()
        .map(|(slug, updated_at)| SitemapEntry { slug, updated_at })
        .collect();
    Ok(ApiResponse::success(entries))
}

// ── Admin API ──

#[utoipa::path(get, path = "/admin/pages", tag = "pages",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "All pages (admin)"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AdminPageListQuery>,
) -> AppResult<ApiResponse<PaginatedData<PageResponse>>> {
    auth.ensure_author()?;
    let pagination = PaginationParams::from_options(query.page, query.page_size);

    let (items, total) = state
        .page_service
        .list_all(pagination.page, pagination.page_size, query.status, &auth)
        .await?;

    let items: Vec<PageResponse> = items.into_iter().map(PageResponse::from_page).collect();
    Ok(pagination.paginate(items, total))
}

#[utoipa::path(get, path = "/admin/pages/{id}", tag = "pages",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Page ID")),
    responses((status = 200, description = "Page details"))
)]
pub async fn admin_get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PageResponse>> {
    auth.ensure_author()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let page = state.page_service.get_by_id(id, &auth).await?;
    Ok(ApiResponse::success(PageResponse::from_page(page)))
}

#[utoipa::path(post, path = "/pages", tag = "pages",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Page created"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreatePageRequest>,
) -> AppResult<ApiResponse<PageResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;

    let slug = req
        .slug
        .unwrap_or_else(|| crate::aspects::slug_aspect::generate_slug(&req.title));
    let template = req.template.unwrap_or_else(|| "default".to_string());
    let status = req.status.unwrap_or(PageStatus::Draft);

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
        created_by: auth.user_id().ok_or(AppError::Unauthorized)?,
        updated_by: None,
        cover_image: req.cover_image,
    };

    let page = state.page_service.create_page(&auth, cmd).await?;
    Ok(ApiResponse::success(PageResponse::from_page(page)))
}

#[utoipa::path(put, path = "/admin/pages/{id}", tag = "pages",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Page ID")),
    responses((status = 200, description = "Page updated"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePageRequest>,
) -> AppResult<ApiResponse<PageResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;

    let resolved_parent_id = resolve_page_parent_id(&state.pool, req.parent_id.flatten()).await?;
    let cmd = UpdatePageCmd {
        id: SnowflakeId(0),
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
        updated_by: auth.user_id(),
    };

    let id = crate::types::snowflake_id::parse_id(&id)?;
    let page = state.page_service.update_page(&auth, id, cmd).await?;
    Ok(ApiResponse::success(PageResponse::from_page(page)))
}

#[utoipa::path(delete, path = "/admin/pages/{id}", tag = "pages",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Page ID")),
    responses((status = 200, description = "Page deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.page_service.delete_page(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(put, path = "/admin/pages/{id}/status", tag = "pages",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Page ID")),
    responses((status = 200, description = "Page status updated"))
)]
pub async fn update_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> AppResult<ApiResponse<PageResponse>> {
    auth.ensure_author()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let page = state
        .page_service
        .update_status(id, req.status, &auth)
        .await?;
    Ok(ApiResponse::success(PageResponse::from_page(page)))
}

#[utoipa::path(put, path = "/admin/pages/reorder", tag = "pages",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Pages reordered"))
)]
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
    state.page_service.reorder(items, &auth).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/pages/batch", tag = "pages",
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
    for raw_id in &req.ids {
        let Ok(id) = crate::types::snowflake_id::parse_id(raw_id) else {
            continue;
        };
        match req.action.as_str() {
            "delete" if state.page_service.delete_page(id, &auth).await.is_ok() => {
                affected += 1;
            }
            "publish" | "unpublish" => {
                let status = if req.action == "publish" {
                    PageStatus::Published
                } else {
                    PageStatus::Draft
                };
                if state
                    .page_service
                    .update_status(id, status, &auth)
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}

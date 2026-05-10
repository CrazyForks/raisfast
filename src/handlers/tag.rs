//! 标签相关处理器

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{BatchRequest, BatchResponse, CreateTagRequest, UpdateTagRequest};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::tag;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post, put};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/tags",
        get(self::list).post(create),
        "system public",
        "tags",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/tags/{id}",
        put(update).delete(self::delete),
        "system public",
        "tags",
        ["PUT", "DELETE"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/tags",
        get(admin_list).post(admin_create),
        "system admin",
        "admin/tags",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/tags/{id}",
        put(admin_update).delete(admin_delete),
        "system admin",
        "admin/tags",
        ["PUT", "DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/tags/batch",
        http_post(admin_batch),
        "system admin",
        "admin/tags",
        ["POST"]
    )
}

/// 获取标签列表（分页）
#[utoipa::path(get, path = "/tags", tag = "tags",
    responses((status = 200, description = "标签列表"))
)]
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::tag::Tag>>> {
    params.sanitize();
    let (items, total) = tag::list_tags_paginated(
        state.tag_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
    )
    .await?;
    Ok(params.paginate(items, total))
}

/// 创建新标签
#[utoipa::path(post, path = "/tags", tag = "tags",
    security(("bearer_auth" = [])),
    request_body = CreateTagRequest,
    responses((status = 200, description = "标签已创建"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let t = tag::create_tag(state.tag_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(t))
}

/// 删除标签
#[utoipa::path(delete, path = "/tags/{id}", tag = "tags",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "标签 ID")),
    responses((status = 200, description = "标签已删除"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    tag::delete_tag(state.tag_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

/// 更新标签
#[utoipa::path(put, path = "/tags/{id}", tag = "tags",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "标签 ID")),
    request_body = UpdateTagRequest,
    responses((status = 200, description = "标签已更新"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let t = tag::update_tag(state.tag_repo.as_ref(), &id, &auth, req.name).await?;
    Ok(ApiResponse::success(t))
}

// ── Admin handlers ──

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::tag::Tag>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = tag::list_tags_paginated(
        state.tag_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
    )
    .await?;
    Ok(params.paginate(items, total))
}

pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let t = tag::create_tag(state.tag_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(t))
}

pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let t = tag::update_tag(state.tag_repo.as_ref(), &id, &auth, req.name).await?;
    Ok(ApiResponse::success(t))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    tag::delete_tag(state.tag_repo.as_ref(), &id, &auth).await?;
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
    if req.action == "delete" {
        for id in &req.ids {
            if tag::delete_tag(state.tag_repo.as_ref(), id, &auth)
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

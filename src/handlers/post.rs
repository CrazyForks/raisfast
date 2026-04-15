//! 文章相关处理器
//!
//! 处理文章的列表、详情、创建、更新和删除请求。
//! 支持按分类、标签、关键词筛选文章列表，以及权限控制（仅作者或管理员可修改/删除）。

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::handlers::dto::{CreatePostRequest, PostResponse, UpdatePostRequest};
use crate::middleware::auth::{AuthUser, AuthorUser};
use crate::middleware::tenant::ResolvedTenant;
use crate::services::post as post_service;
use crate::utils::pagination::PaginationParams;

/// 文章列表查询参数
///
/// 支持分页、按分类/标签筛选、关键词搜索。
#[derive(Debug, Deserialize, Default)]
pub struct PostListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub category_id: Option<String>,
    pub tag_id: Option<String>,
    pub q: Option<String>,
}

/// 后台管理文章列表查询参数
#[derive(Debug, Deserialize, Default)]
pub struct AdminPostListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
}

/// 获取已发布文章列表（分页）
///
/// - **方法/路径：** `GET /api/posts`
/// - **认证：** 无需认证
/// - **说明：** 分页查询已发布文章，支持按 `category_id`、`tag_id`、`q`（关键词）筛选。
///   `page_size` 上限为 100。
/// - **返回：** `ApiResponse<PaginatedData<PostResponse>>`
pub async fn list(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Query(query): Query<PostListQuery>,
) -> AppResult<ApiResponse<PaginatedData<PostResponse>>> {
    let mut pagination = PaginationParams::default();
    if let Some(page) = query.page {
        pagination.page = page.max(1);
    }
    if let Some(page_size) = query.page_size {
        pagination.page_size = page_size.clamp(1, 100);
    }
    pagination.sanitize();

    let (posts, total) = post_service::list_posts(
        state.post_repo.as_ref(),
        pagination.page,
        pagination.page_size,
        query.category_id.as_deref(),
        query.tag_id.as_deref(),
        query.q.as_deref(),
        &state.plugins,
        Some(state.search.as_ref()),
        tenant.as_str(),
    )
    .await?;

    Ok(pagination.paginate(posts, total))
}

/// 获取文章详情（按 slug）
///
/// - **方法/路径：** `GET /api/posts/:slug`
/// - **认证：** 无需认证
/// - **说明：** 根据 slug 获取已发布文章详情，自动增加浏览量。
/// - **返回：** `ApiResponse<PostResponse>`
pub async fn get(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<PostResponse>> {
    let post = post_service::get_post(
        state.post_repo.as_ref(),
        &slug,
        &state.plugins,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// 创建新文章
///
/// - **方法/路径：** `POST /api/posts`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 创建新文章，自动生成 slug，支持设置分类和标签。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<PostResponse>`
pub async fn create(
    State(state): State<crate::AppState>,
    author: AuthorUser,
    tenant: ResolvedTenant,
    Json(req): Json<CreatePostRequest>,
) -> AppResult<ApiResponse<PostResponse>> {
    validation::validate(&req)?;
    let post = post_service::create_post(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &author.user_id,
        req,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// 更新文章
///
/// - **方法/路径：** `PUT /api/posts/:slug`
/// - **认证：** 需要登录（`AuthUser`），且为文章作者或管理员
/// - **说明：** 根据 slug 查找文章，验证权限后更新。仅修改提供的字段。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<PostResponse>`
pub async fn update(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
    Path(slug): Path<String>,
    Json(req): Json<UpdatePostRequest>,
) -> AppResult<ApiResponse<PostResponse>> {
    validation::validate(&req)?;
    let post = post_service::update_post_with_auth(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        &auth_user.user_id,
        &auth_user.role,
        req,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// 删除文章
///
/// - **方法/路径：** `DELETE /api/posts/:slug`
/// - **认证：** 需要登录（`AuthUser`），且为文章作者或管理员
/// - **说明：** 根据 slug 查找文章，验证权限后删除。
/// - **返回：** `ApiResponse<()>`
pub async fn delete(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<()>> {
    post_service::delete_post_with_auth(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        &auth_user.user_id,
        &auth_user.role,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 后台管理：按 slug 获取文章详情（含所有状态）
///
/// - **方法/路径：** `GET /api/v1/admin/posts/{slug}`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 根据 slug 获取任意状态的文章详情，不增加浏览量。
/// - **返回：** `ApiResponse<PostResponse>`
pub async fn admin_get(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<PostResponse>> {
    let post = post_service::get_post_any_status(
        state.post_repo.as_ref(),
        &slug,
        &state.plugins,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// 后台管理：获取全部文章列表（含所有状态）
///
/// - **方法/路径：** `GET /api/v1/admin/posts`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 分页查询全部文章（含 draft/published），支持按 `status` 筛选。
/// - **返回：** `ApiResponse<PaginatedData<PostResponse>>`
pub async fn admin_list(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Query(query): Query<AdminPostListQuery>,
) -> AppResult<ApiResponse<PaginatedData<PostResponse>>> {
    let mut pagination = PaginationParams::default();
    if let Some(page) = query.page {
        pagination.page = page.max(1);
    }
    if let Some(page_size) = query.page_size {
        pagination.page_size = page_size.clamp(1, 100);
    }
    pagination.sanitize();

    let (posts, total) = post_service::list_all_posts(
        state.post_repo.as_ref(),
        pagination.page,
        pagination.page_size,
        query.status.as_deref(),
        &state.plugins,
        tenant.as_str(),
    )
    .await?;

    Ok(pagination.paginate(posts, total))
}

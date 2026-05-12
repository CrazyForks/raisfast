//! Post handlers
//!
//! Handles post list, detail, create, update, and delete requests.
//! Supports filtering by category, tag, and keyword, with permission control
//! (only the author or admin can modify/delete).

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::dto::{BatchRequest, BatchResponse, CreatePostRequest, PostResponse, UpdatePostRequest};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::post::PostStatus;
use crate::services::post as post_service;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/posts",
        get(self::list).post(create),
        "system public",
        "posts",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/posts/{slug}",
        get(self::get).put(update).delete(self::delete),
        "system public",
        "posts",
        ["GET", "PUT", "DELETE"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/posts",
        get(admin_list).post(admin_create),
        "system admin",
        "admin/posts",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/posts/{id}",
        get(admin_get).put(admin_update).delete(admin_delete),
        "system admin",
        "admin/posts",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/posts/batch",
        http_post(admin_batch),
        "system admin",
        "admin/posts",
        ["POST"]
    )
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Default)]
pub struct PostListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub category_id: Option<String>,
    pub tag_id: Option<String>,
    pub q: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Default)]
pub struct AdminPostListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<PostStatus>,
}

/// Get published post list (paginated)
///
/// - **Method/Path:** `GET /api/posts`
/// - **Auth:** Not required
/// - **Description:** Paginated query of published posts, supports filtering by `category_id`, `tag_id`, `q` (keyword).
///   `page_size` upper limit is 100.
/// - **Returns:** `ApiResponse<PaginatedData<PostResponse>>`
#[utoipa::path(get, path = "/posts", tag = "posts",
    responses((status = 200, description = "Post list"))
)]
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<PostListQuery>,
) -> AppResult<ApiResponse<PaginatedData<PostResponse>>> {
    let pagination = PaginationParams::from_options(query.page, query.page_size);

    let cat_id = if let Some(ref cid) = query.category_id {
        post_service::resolve_doc_id_to_int(&state.pool, "categories", cid, None).await?
    } else {
        None
    };
    let tg_id = if let Some(ref tid) = query.tag_id {
        post_service::resolve_doc_id_to_int(&state.pool, "tags", tid, None).await?
    } else {
        None
    };

    let (posts, total) = post_service::list_posts(
        state.post_repo.as_ref(),
        pagination.page,
        pagination.page_size,
        cat_id,
        tg_id,
        query.q.as_deref(),
        &state.plugins,
        Some(state.search.as_ref()),
        &auth,
    )
    .await?;

    Ok(pagination.paginate(posts, total))
}

/// Get post detail (by slug)
///
/// - **Method/Path:** `GET /api/posts/:slug`
/// - **Auth:** Not required
/// - **Description:** Fetches published post detail by slug, auto-increments view count.
/// - **Returns:** `ApiResponse<PostResponse>`
#[utoipa::path(get, path = "/posts/{slug}", tag = "posts",
    params(("slug" = String, Path, description = "Post slug")),
    responses((status = 200, description = "Post detail"))
)]
pub async fn get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<PostResponse>> {
    let post =
        post_service::get_post(state.post_repo.as_ref(), &slug, &state.plugins, &auth).await?;
    Ok(ApiResponse::success(post))
}

/// Create a new post
///
/// - **Method/Path:** `POST /api/posts`
/// - **Auth:** Requires author or above permission (`AuthorUser`)
/// - **Description:** Creates a new post, auto-generates slug, supports setting category and tags.
/// - **Validation:** Validates request body via `validation::validate()`, with i18n error messages.
/// - **Returns:** `ApiResponse<PostResponse>`
#[utoipa::path(post, path = "/posts", tag = "posts",
    security(("bearer_auth" = [])),
    request_body = CreatePostRequest,
    responses((status = 200, description = "Post created"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreatePostRequest>,
) -> AppResult<ApiResponse<PostResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let post = post_service::create_post(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &auth,
        req,
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// Update a post
///
/// - **Method/Path:** `PUT /api/posts/:slug`
/// - **Auth:** Requires login (`AuthUser`), must be post author or admin
/// - **Description:** Finds post by slug, verifies permission, then updates. Only modifies provided fields.
/// - **Validation:** Validates request body via `validation::validate()`, with i18n error messages.
/// - **Returns:** `ApiResponse<PostResponse>`
#[utoipa::path(put, path = "/posts/{slug}", tag = "posts",
    security(("bearer_auth" = [])),
    params(("slug" = String, Path, description = "Post slug")),
    request_body = UpdatePostRequest,
    responses((status = 200, description = "Post updated"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
    Json(req): Json<UpdatePostRequest>,
) -> AppResult<ApiResponse<PostResponse>> {
    validation::validate(&req)?;
    let post = post_service::update_post(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        &auth,
        req,
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// Delete a post
///
/// - **Method/Path:** `DELETE /api/posts/:slug`
/// - **Auth:** Requires login (`AuthUser`), must be post author or admin
/// - **Description:** Finds post by slug, verifies permission, then deletes.
/// - **Returns:** `ApiResponse<()>`
#[utoipa::path(delete, path = "/posts/{slug}", tag = "posts",
    security(("bearer_auth" = [])),
    params(("slug" = String, Path, description = "Post slug")),
    responses((status = 200, description = "Post deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<()>> {
    post_service::delete_post(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        &auth,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

// ── Admin handlers ──

/// Admin: create post (admin, can specify author, force publish)
pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreatePostRequest>,
) -> AppResult<ApiResponse<PostResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let post = post_service::create_post(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &auth,
        req,
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// Admin: edit any post (admin, can change author/status)
pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePostRequest>,
) -> AppResult<ApiResponse<PostResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let post = post_service::admin_update_post(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &id,
        &auth,
        req,
    )
    .await?;
    Ok(ApiResponse::success(post))
}

/// Admin: delete any post (admin)
pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    post_service::admin_delete_post(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &id,
        &auth,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// Admin: get post detail by slug (all statuses included)
///
/// - **Method/Path:** `GET /api/v1/admin/posts/{slug}`
/// - **Auth:** Requires author or above permission (`AuthorUser`)
/// - **Description:** Fetches post detail of any status by slug, without incrementing view count.
/// - **Returns:** `ApiResponse<PostResponse>`
pub async fn admin_get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<PostResponse>> {
    auth.ensure_author()?;
    let post =
        post_service::get_post_any_status(state.post_repo.as_ref(), &slug, &state.plugins, &auth)
            .await?;
    Ok(ApiResponse::success(post))
}

/// Admin: get all posts list (all statuses included)
///
/// - **Method/Path:** `GET /api/v1/admin/posts`
/// - **Auth:** Requires author or above permission (`AuthorUser`)
/// - **Description:** Paginated query of all posts (including draft/published), supports filtering by `status`.
/// - **Returns:** `ApiResponse<PaginatedData<PostResponse>>`
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AdminPostListQuery>,
) -> AppResult<ApiResponse<PaginatedData<PostResponse>>> {
    auth.ensure_author()?;
    let pagination = PaginationParams::from_options(query.page, query.page_size);

    let (posts, total) = post_service::list_posts(
        state.post_repo.as_ref(),
        pagination.page,
        pagination.page_size,
        None,
        None,
        None,
        &state.plugins,
        Some(state.search.as_ref()),
        &auth,
    )
    .await?;

    Ok(pagination.paginate(posts, total))
}

/// Admin: batch operations (delete / publish / unpublish)
pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let affected = post_service::batch_posts(
        state.post_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &req.action,
        &req.ids,
        &auth,
    )
    .await?;
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}

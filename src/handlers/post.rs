//! Post handlers
//!
//! Handles post list, detail, create, update, and delete requests.
//! Supports filtering by category, tag, and keyword, with permission control
//! (only the author or admin can modify/delete).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;

use crate::dto::{
    AdminPostListQuery, BatchRequest, BatchResponse, CreatePostRequest, PostListQuery,
    PostResponse, UpdatePostRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::utils::pagination::PaginationParams;

fn post_list_cache_key(
    page: i64,
    page_size: i64,
    cat_id: Option<i64>,
    tg_id: Option<i64>,
    q: Option<&str>,
    tenant_id: Option<&str>,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    page.hash(&mut h);
    page_size.hash(&mut h);
    cat_id.hash(&mut h);
    tg_id.hash(&mut h);
    q.hash(&mut h);
    tenant_id.hash(&mut h);
    format!("posts:list:{:x}", h.finish())
}

fn invalidate_post_cache(state: &crate::AppState) {
    state.cms_cache.retain(|k, _| !k.starts_with("posts:"));
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
        "/posts",
        get,
        self::list,
        "system public",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/posts",
        create,
        self::create,
        "system public",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/posts/{slug}",
        get,
        self::get,
        "system public",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/posts/{slug}",
        put,
        self::update,
        "system public",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/posts/{slug}",
        delete,
        self::delete,
        "system public",
        "posts"
    );
    r
}

#[allow(clippy::let_and_return)]
pub fn admin_routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/posts",
        get,
        self::admin_list,
        "system admin",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/posts",
        post,
        self::admin_create,
        "system admin",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/posts/{id}",
        get,
        self::admin_get,
        "system admin",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/posts/{id}",
        put,
        self::admin_update,
        "system admin",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/posts/{id}",
        delete,
        self::admin_delete,
        "system admin",
        "posts"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/posts/batch",
        post,
        self::admin_batch,
        "system admin",
        "posts"
    );
    r
}

/// List published posts
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
) -> AppResult<axum::response::Response> {
    let pagination = PaginationParams::from_options(query.page, query.page_size);

    let cat_id = if let Some(ref cid) = query.category_id {
        let parsed = crate::types::snowflake_id::parse_id(cid)?;
        raisfast_derive::crud_resolve_id!(&state.pool, "categories", *parsed)?
    } else {
        None
    };
    let tg_id = if let Some(ref tid) = query.tag_id {
        let parsed = crate::types::snowflake_id::parse_id(tid)?;
        raisfast_derive::crud_resolve_id!(&state.pool, "tags", *parsed)?
    } else {
        None
    };

    let cache_key = post_list_cache_key(
        pagination.page,
        pagination.page_size,
        cat_id,
        tg_id,
        query.q.as_deref(),
        auth.tenant_id(),
    );
    let cache_ttl = std::time::Duration::from_secs(state.config.rule_engine.cms_cache_ttl_secs);
    if let Some(entry) = state.cms_cache.get(&cache_key)
        && entry.value().1.elapsed() < cache_ttl
    {
        return Ok(axum::Json(entry.value().0.clone()).into_response());
    }

    let (posts, total) = state
        .post_service
        .list(
            &auth,
            pagination.page,
            pagination.page_size,
            cat_id,
            tg_id,
            query.q.as_deref(),
        )
        .await?;

    let resp = pagination.paginate(posts, total);
    if let Ok(val) = serde_json::to_value(&resp) {
        state
            .cms_cache
            .insert(cache_key, (val, std::time::Instant::now()));
    }

    Ok(resp.into_response())
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
) -> AppResult<axum::response::Response> {
    let post = state.post_service.get(&auth, &slug).await?;
    Ok(ApiResponse::success(post).into_response())
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
    let post = state.post_service.create(&auth, req).await?;
    invalidate_post_cache(&state);
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
    auth.ensure_author()?;
    validation::validate(&req)?;
    let post = state.post_service.update(&auth, &slug, req).await?;
    invalidate_post_cache(&state);
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
    auth.ensure_author()?;
    state.post_service.delete(&auth, &slug).await?;
    invalidate_post_cache(&state);
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
    let post = state.post_service.create(&auth, req).await?;
    invalidate_post_cache(&state);
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let post = state.post_service.admin_update(&auth, id, req).await?;
    invalidate_post_cache(&state);
    Ok(ApiResponse::success(post))
}

/// Admin: delete any post (admin)
pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.post_service.admin_delete(&auth, id).await?;
    invalidate_post_cache(&state);
    Ok(ApiResponse::success(()))
}

/// Admin: get post detail by id (all statuses included)
///
/// - **Method/Path:** `GET /api/v1/admin/posts/{id}`
/// - **Auth:** Requires author or above permission (`AuthorUser`)
/// - **Description:** Fetches post detail of any status by id, without incrementing view count.
/// - **Returns:** `ApiResponse<PostResponse>`
pub async fn admin_get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PostResponse>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let post = state.post_service.admin_get_by_id(&auth, id).await?;
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
    auth.ensure_admin()?;
    let pagination = PaginationParams::from_options(query.page, query.page_size);

    let (posts, total) = state
        .post_service
        .list_all(
            &auth,
            pagination.page,
            pagination.page_size,
            query.status,
            query.keyword.as_deref(),
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
    let affected = state
        .post_service
        .batch(&auth, &req.action, &req.ids)
        .await?;
    invalidate_post_cache(&state);
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}

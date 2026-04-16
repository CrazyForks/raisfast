//! 泛型内容类型 API Handler
//!
//! 为所有注册的 content type 自动提供 CRUD HTTP 端点。
//! 启动时注册已知 content type 的固定路由；启动后新增的 content type 通过
//! catch-all 动态路由处理。两种路由共享同一套核心逻辑。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

use super::repository::{ContentQuery, ContentRepository};
use super::schema::{ContentTypeSchema, check_api_access};
use crate::AppState;
use crate::errors::app_error::AppError;
use crate::middleware::auth::OptionalAuth;

/// 统一 API 响应
#[derive(Debug, serde::Serialize)]
pub struct ApiResponse<T: serde::Serialize> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
}

impl<T: serde::Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            code: 0,
            message: "success".into(),
            data: Some(data),
        }
    }

    #[must_use]
    pub fn error(code: i32, message: String) -> ApiResponse<()> {
        ApiResponse {
            code,
            message,
            data: None,
        }
    }
}

/// 分页查询参数
#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub sort: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub include: Option<String>,
}

/// 为所有 content type 注册动态路由（启动时调用）
pub fn register_content_routes(
    router: axum::Router<AppState>,
    registry: &crate::content_type::ContentTypeRegistry,
) -> axum::Router<AppState> {
    let mut api = router;

    for ct in registry.all() {
        let plural = ct.plural.clone();
        let singular = ct.singular.clone();

        api = api
            .route(
                &format!("/cms/{plural}"),
                axum::routing::get({
                    let singular = singular.clone();
                    move |auth, state, params| list_handler(auth, state, singular.clone(), params)
                })
                .post({
                    let singular = singular.clone();
                    move |auth, state, data| create_handler(auth, state, singular.clone(), data)
                }),
            )
            .route(
                &format!("/cms/{plural}/{{id_or_slug}}"),
                axum::routing::get({
                    let singular = singular.clone();
                    move |auth, state, path| get_handler(auth, state, path, singular.clone())
                })
                .put({
                    let singular = singular.clone();
                    move |auth, state, path, data| {
                        update_handler(auth, state, path, data, singular.clone())
                    }
                })
                .delete({
                    let singular = singular.clone();
                    move |auth, state, path| delete_handler(auth, state, path, singular.clone())
                }),
            )
            .route(
                &format!("/admin/cms/{plural}"),
                axum::routing::get({
                    let singular = singular.clone();
                    move |state, params| admin_list_handler(state, singular.clone(), params)
                }),
            )
            .route(
                &format!("/admin/cms/{plural}/{{id_or_slug}}"),
                axum::routing::get({
                    let singular = singular.clone();
                    move |state, path| admin_get_handler(state, path, singular.clone())
                }),
            );

        tracing::debug!("registered CMS routes for content type: {}", ct.singular);
    }

    api
}

/// 解析 catch-all 路径为 (plural, 可选 id_or_slug)
fn parse_dynamic_path(path: &str) -> Option<(String, Option<String>)> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    let plural = segments[0].to_string();
    let id = segments.get(1).map(|s| s.to_string());
    Some((plural, id))
}

/// Catch-all 动态路由 handler（启动后新增的 content type 走这里）
pub async fn dynamic_cms_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, AppError> {
    let Some((plural, id)) = parse_dynamic_path(&path) else {
        return Err(AppError::not_found("invalid cms path"));
    };

    let Some(ct) = state.content_type_registry.get_by_plural(&plural) else {
        return Err(AppError::not_found(&plural));
    };

    match (method.clone(), id) {
        (axum::http::Method::GET, None) => {
            check_api_access(ct.api.list, auth.0.as_ref())?;
            let data = do_list(&state, &ct, params).await?;
            Ok(Json(data).into_response())
        }
        (axum::http::Method::POST, None) => {
            check_api_access(ct.api.create, auth.0.as_ref())?;
            let Json(data) = body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
            let result = do_create(&state, &ct, data).await?;
            Ok((StatusCode::CREATED, Json(result)).into_response())
        }
        (axum::http::Method::GET, Some(id)) => {
            check_api_access(ct.api.get, auth.0.as_ref())?;
            let data = do_get(&state, &ct, &id).await?;
            Ok(Json(data).into_response())
        }
        (axum::http::Method::PUT, Some(id)) => {
            check_api_access(ct.api.update, auth.0.as_ref())?;
            let Json(data) = body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
            let result = do_update(&state, &ct, &id, data).await?;
            Ok(Json(result).into_response())
        }
        (axum::http::Method::DELETE, Some(id)) => {
            check_api_access(ct.api.delete, auth.0.as_ref())?;
            do_delete(&state, &ct, &id).await?;
            Ok(Json(json!({"deleted": true})).into_response())
        }
        _ => Err(AppError::not_found(&format!("{method} {path}"))),
    }
}

/// Catch-all admin 动态路由 handler
pub async fn dynamic_admin_cms_handler(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
    let Some((plural, id)) = parse_dynamic_path(&path) else {
        return Err(AppError::not_found("invalid admin cms path"));
    };

    let Some(ct) = state.content_type_registry.get_by_plural(&plural) else {
        return Err(AppError::not_found(&plural));
    };

    match (method.clone(), id) {
        (axum::http::Method::GET, None) => {
            let data = do_admin_list(&state, &ct, params).await?;
            Ok(Json(data).into_response())
        }
        (axum::http::Method::GET, Some(id)) => {
            let data = do_admin_get(&state, &ct, &id).await?;
            Ok(Json(data).into_response())
        }
        _ => Err(AppError::not_found(&format!("{method} {path}"))),
    }
}

// ── 核心业务逻辑（共享于固定路由和动态路由） ──────────────────────

async fn do_list(
    state: &AppState,
    ct: &ContentTypeSchema,
    params: ListParams,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    let include = params.include.as_deref().map(parse_include);
    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters: HashMap::new(),
        status: if ct.draft_publish {
            Some(params.status.unwrap_or_else(|| "published".into()))
        } else {
            None
        },
        search: params.search,
        fields: None,
        tenant_id: None,
        include,
    };
    let (items, total) = repo.find(ct, query.clone()).await?;
    Ok(json!({
        "items": items,
        "total": total,
        "page": query.page,
        "page_size": query.page_size,
    }))
}

async fn do_get(
    state: &AppState,
    ct: &ContentTypeSchema,
    id_or_slug: &str,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    let status = if ct.draft_publish {
        Some("published")
    } else {
        None
    };

    let item = if id_or_slug.contains('-') && !id_or_slug.contains('/') {
        repo.find_by_slug(ct, id_or_slug, status, None)
            .await?
            .or(None)
    } else {
        None
    };

    let item = match item {
        Some(data) => Some(data),
        None => repo.find_by_id(ct, id_or_slug, None).await?,
    };

    item.ok_or_else(|| AppError::not_found(&format!("{}/{}", ct.name, id_or_slug)))
}

async fn do_create(
    state: &AppState,
    ct: &ContentTypeSchema,
    data: Value,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    repo.create(ct, data, None).await
}

async fn do_update(
    state: &AppState,
    ct: &ContentTypeSchema,
    id: &str,
    data: Value,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    repo.update(ct, id, data, None).await
}

async fn do_delete(state: &AppState, ct: &ContentTypeSchema, id: &str) -> Result<(), AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    repo.delete(ct, id, None).await
}

async fn do_admin_list(
    state: &AppState,
    ct: &ContentTypeSchema,
    params: ListParams,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    let include = params.include.as_deref().map(parse_include);
    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters: HashMap::new(),
        status: params.status,
        search: params.search,
        fields: None,
        tenant_id: None,
        include,
    };
    let (items, total) = repo.find(ct, query.clone()).await?;
    Ok(json!({
        "items": items,
        "total": total,
        "page": query.page,
        "page_size": query.page_size,
    }))
}

async fn do_admin_get(
    state: &AppState,
    ct: &ContentTypeSchema,
    id: &str,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    let item = repo.find_by_id(ct, id, None).await?;
    item.ok_or_else(|| AppError::not_found(&format!("{}/{}", ct.name, id)))
}

// ── 固定路由 handler（启动时注册的 content type） ──────────────

async fn list_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    type_name: String,
    Query(params): Query<ListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.list, auth.0.as_ref())?;
    let data = do_list(&state, &ct, params).await?;
    Ok(Json(data))
}

async fn get_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    Path(id_or_slug): Path<String>,
    type_name: String,
) -> Result<Json<serde_json::Value>, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.get, auth.0.as_ref())?;
    let data = do_get(&state, &ct, &id_or_slug).await?;
    Ok(Json(data))
}

async fn create_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    type_name: String,
    Json(data): Json<Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.create, auth.0.as_ref())?;
    let result = do_create(&state, &ct, data).await?;
    Ok((StatusCode::CREATED, Json(result)))
}

async fn update_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(data): Json<Value>,
    type_name: String,
) -> Result<Json<serde_json::Value>, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.update, auth.0.as_ref())?;
    let result = do_update(&state, &ct, &id, data).await?;
    Ok(Json(result))
}

async fn delete_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    type_name: String,
) -> Result<Json<serde_json::Value>, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.delete, auth.0.as_ref())?;
    do_delete(&state, &ct, &id).await?;
    Ok(Json(json!({"deleted": true})))
}

async fn admin_list_handler(
    State(state): State<AppState>,
    type_name: String,
    Query(params): Query<ListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let data = do_admin_list(&state, &ct, params).await?;
    Ok(Json(data))
}

async fn admin_get_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    type_name: String,
) -> Result<Json<serde_json::Value>, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let data = do_admin_get(&state, &ct, &id).await?;
    Ok(Json(data))
}

// ── Schema 管理 API ──────────────────────────────────────────

/// GET /admin/content-types — 列出所有已注册 content type 的 schema 定义
pub async fn list_schemas(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let schemas = state.content_type_registry.all();
    Ok(Json(crate::errors::response::ApiResponse::success(schemas)))
}

/// GET /admin/content-types/:singular — 获取单个 content type 的 schema 定义
pub async fn get_schema(
    State(state): State<AppState>,
    Path(singular): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&singular)
        .ok_or_else(|| AppError::not_found(&singular))?;
    Ok(Json(crate::errors::response::ApiResponse::success(ct)))
}

/// POST /admin/content-types — 创建新的 content type
///
/// 1. 校验 singular 唯一性
/// 2. 写 TOML 文件
/// 3. 执行 DB migration（建表/加列）
/// 4. 注册到内存 ContentTypeRegistry（立即生效，无需重启）
pub async fn create_schema(
    State(state): State<AppState>,
    Json(req): Json<super::schema::CreateContentTypeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let schema = super::schema::ContentTypeSchema {
        extension_id: None,
        name: req.name,
        singular: req.singular.clone(),
        plural: req.plural,
        table: req.table,
        description: req.description,
        draft_publish: req.draft_publish,
        slug_field: req.slug_field,
        timestamps: req.timestamps,
        soft_delete: req.soft_delete,
        fields: req.fields,
        indexes: vec![],
        list_view: None,
        api: super::schema::ApiConfig::default(),
    };

    if state.content_type_registry.get(&req.singular).is_some() {
        return Err(AppError::Conflict(format!(
            "content type '{}' already exists",
            req.singular
        )));
    }

    let dir = std::path::Path::new(&state.config.content_type_dir);
    schema.save_to_dir(dir)?;

    let repo = ContentRepository::new(state.pool.clone());
    repo.migrate(&schema).await?;

    state.content_type_registry.register(schema.clone());

    tracing::info!(
        "registered content type: {} (table={}, hot-reload)",
        schema.singular,
        schema.table
    );

    Ok((
        StatusCode::CREATED,
        Json(crate::errors::response::ApiResponse::success(schema)),
    ))
}

/// DELETE /admin/content-types/:singular — 删除 content type
///
/// 删除 TOML 文件 + 从内存注册表注销。不删除数据库表。
pub async fn delete_schema(
    State(state): State<AppState>,
    Path(singular): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if state.content_type_registry.get(&singular).is_none() {
        return Err(AppError::not_found(&singular));
    }

    let path =
        std::path::Path::new(&state.config.content_type_dir).join(format!("{singular}.toml"));
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("cannot delete {:?}: {e}", path)))?;
    }

    state.content_type_registry.unregister(&singular);

    tracing::info!("unregistered content type: {} (hot-reload)", singular);

    Ok(Json(crate::errors::response::ApiResponse::success(
        serde_json::json!({"deleted": true}),
    )))
}

/// PUT /admin/content-types/:singular — 更新 content type schema
///
/// 增量更新：只修改请求中提供的字段。如果提供了 `fields`，会与数据库对比并
/// 自动 `ALTER TABLE ADD COLUMN` 补齐新增列（不删除列、不改列类型）。
/// 更新后的 schema 同步写入内存注册表（立即生效，无需重启）。
pub async fn update_schema(
    State(state): State<AppState>,
    Path(singular): Path<String>,
    Json(req): Json<super::schema::UpdateContentTypeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&singular)
        .ok_or_else(|| AppError::not_found(&singular))?;

    let mut updated = ct;

    if let Some(name) = req.name {
        updated.name = name;
    }
    if let Some(description) = req.description {
        updated.description = description;
    }
    if let Some(draft_publish) = req.draft_publish {
        updated.draft_publish = draft_publish;
    }
    if let Some(slug_field) = req.slug_field {
        updated.slug_field = slug_field;
    }
    if let Some(timestamps) = req.timestamps {
        updated.timestamps = timestamps;
    }
    if let Some(soft_delete) = req.soft_delete {
        updated.soft_delete = soft_delete;
    }
    if let Some(fields) = req.fields {
        updated.fields = fields;
    }
    if let Some(indexes) = req.indexes {
        updated.indexes = indexes;
    }
    if let Some(list_view) = req.list_view {
        updated.list_view = list_view;
    }

    let dir = std::path::Path::new(&state.config.content_type_dir);
    updated.save_to_dir(dir)?;

    let repo = ContentRepository::new(state.pool.clone());
    repo.migrate(&updated).await?;

    state.content_type_registry.register(updated.clone());

    tracing::info!(
        "updated content type: {} (table={}, hot-reload)",
        updated.singular,
        updated.table
    );

    Ok(Json(crate::errors::response::ApiResponse::success(updated)))
}

fn parse_include(s: &str) -> Vec<String> {
    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

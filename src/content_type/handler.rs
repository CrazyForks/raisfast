//! 泛型内容类型 API Handler
//!
//! 为所有注册的 content type 自动提供 CRUD HTTP 端点。
//! 路由路径中的 `{plural}` 段在注册时确定，通过闭包捕获传入 handler。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

use super::repository::{ContentQuery, ContentRepository};
use crate::AppState;
use crate::content_type::ContentTypeRegistry;
use crate::errors::app_error::AppError;

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
}

/// 为所有 content type 注册动态路由
pub fn register_content_routes(
    router: axum::Router<AppState>,
    registry: &std::sync::Arc<ContentTypeRegistry>,
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
                    move |state, params| list_handler(state, singular.clone(), params)
                })
                .post({
                    let singular = singular.clone();
                    move |state, data| create_handler(state, singular.clone(), data)
                }),
            )
            .route(
                &format!("/cms/{plural}/{{id_or_slug}}"),
                axum::routing::get({
                    let singular = singular.clone();
                    move |state, path| get_handler(state, path, singular.clone())
                })
                .put({
                    let singular = singular.clone();
                    move |state, path, data| update_handler(state, path, data, singular.clone())
                })
                .delete({
                    let singular = singular.clone();
                    move |state, path| delete_handler(state, path, singular.clone())
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

async fn list_handler(
    State(state): State<AppState>,
    type_name: String,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;

    let repo = ContentRepository::new(state.pool.clone());

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
    };

    let (items, total) = repo.find(ct, query.clone()).await?;

    Ok(Json(ApiResponse::success(json!({
        "items": items,
        "total": total,
        "page": query.page,
        "page_size": query.page_size,
    }))))
}

async fn get_handler(
    State(state): State<AppState>,
    Path(id_or_slug): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;

    let repo = ContentRepository::new(state.pool.clone());

    let status = if ct.draft_publish {
        Some("published")
    } else {
        None
    };

    let item = if id_or_slug.contains('-') && !id_or_slug.contains('/') {
        repo.find_by_slug(ct, &id_or_slug, status, None)
            .await?
            .or(None)
    } else {
        None
    };

    let item = match item {
        Some(data) => Some(data),
        None => repo.find_by_id(ct, &id_or_slug, None).await?,
    };

    match item {
        Some(data) => Ok(Json(ApiResponse::success(data))),
        None => Err(AppError::not_found(&format!("{}/{}", ct.name, id_or_slug))),
    }
}

async fn create_handler(
    State(state): State<AppState>,
    type_name: String,
    Json(data): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;

    let repo = ContentRepository::new(state.pool.clone());
    let result = repo.create(ct, data, None).await?;

    Ok((StatusCode::CREATED, Json(ApiResponse::success(result))))
}

async fn update_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(data): Json<Value>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;

    let repo = ContentRepository::new(state.pool.clone());
    let result = repo.update(ct, &id, data, None).await?;

    Ok(Json(ApiResponse::success(result)))
}

async fn delete_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;

    let repo = ContentRepository::new(state.pool.clone());
    repo.delete(ct, &id, None).await?;

    Ok(Json(ApiResponse::success(json!({"deleted": true}))))
}

async fn admin_list_handler(
    State(state): State<AppState>,
    type_name: String,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;

    let repo = ContentRepository::new(state.pool.clone());

    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters: HashMap::new(),
        status: params.status,
        search: params.search,
        fields: None,
        tenant_id: None,
    };

    let (items, total) = repo.find(ct, query.clone()).await?;

    Ok(Json(ApiResponse::success(json!({
        "items": items,
        "total": total,
        "page": query.page,
        "page_size": query.page_size,
    }))))
}

async fn admin_get_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;

    let repo = ContentRepository::new(state.pool.clone());
    let item = repo.find_by_id(ct, &id, None).await?;

    match item {
        Some(data) => Ok(Json(ApiResponse::success(data))),
        None => Err(AppError::not_found(&format!("{}/{}", ct.name, id))),
    }
}

/// GET /admin/content-types — 列出所有已注册 content type 的 schema 定义
pub async fn list_schemas(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let schemas: Vec<_> = state.content_type_registry.all().into_iter().cloned().collect();
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
    Ok(Json(crate::errors::response::ApiResponse::success(ct.clone())))
}

/// POST /admin/content-types — 创建新的 content type
pub async fn create_schema(
    State(state): State<AppState>,
    Json(req): Json<super::schema::CreateContentTypeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let schema = super::schema::ContentTypeSchema {
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

    Ok((
        StatusCode::CREATED,
        Json(crate::errors::response::ApiResponse::success(schema)),
    ))
}

/// DELETE /admin/content-types/:singular — 删除 content type
pub async fn delete_schema(
    State(state): State<AppState>,
    Path(singular): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let _ct = state
        .content_type_registry
        .get(&singular)
        .ok_or_else(|| AppError::not_found(&singular))?;

    let path =
        std::path::Path::new(&state.config.content_type_dir).join(format!("{singular}.toml"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("cannot delete {:?}: {e}", path))
        })?;
    }

    Ok(Json(crate::errors::response::ApiResponse::success(
        serde_json::json!({"deleted": true}),
    )))
}

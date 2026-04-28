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

use super::repository::{ContentQuery, ContentRepository, SaveContext};
use super::rule_engine::compile_rule_sql;
use super::schema::{ContentTypeSchema, check_api_access};
use crate::AppState;
use crate::errors::app_error::AppError;
use crate::middleware::auth::OptionalAuth;

/// 编译 API Rule 为 SQL WHERE 子句。
///
/// 如果用户已登录且有 `filter_auth`，生成 `filter OR filter_auth`；
/// 否则只用 `filter`。
/// 如果规则需要认证但用户未登录，返回 None（调用方应拒绝请求）。
fn build_rule_sql(
    endpoint: &super::schema::CachedEndpointRules,
    auth: Option<&crate::middleware::auth::AuthIdentity>,
    config: &crate::config::app::RuleEngineConfig,
) -> Option<(String, Vec<String>)> {
    match (
        endpoint.filter.as_ref(),
        endpoint.filter_auth.as_ref(),
        auth,
    ) {
        (None, None, _) => None,
        (Some(rule), None, _) => compile_rule_sql(rule, 0, None, config),
        (None, Some(_), None) => None,
        (Some(rule), Some(_), None) => compile_rule_sql(rule, 0, None, config),
        (None, Some(auth_rule), Some(a)) => compile_rule_sql(auth_rule, 0, Some(a), config),
        (Some(rule), Some(auth_rule), Some(a)) => {
            let (base_sql, mut base_params) = compile_rule_sql(rule, 0, None, config)?;
            let offset = base_params.len();
            let (auth_sql, mut auth_params) = compile_rule_sql(auth_rule, offset, Some(a), config)?;
            let combined = format!("({base_sql} OR {auth_sql})");
            base_params.append(&mut auth_params);
            Some((combined, base_params))
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
    /// 跳过 COUNT(*) 查询以提升性能，total 返回 -1
    #[serde(default)]
    pub skip_total: Option<bool>,
    /// 额外字段过滤（匹配 content type schema 字段名）
    #[serde(flatten)]
    pub extra: HashMap<String, String>,
}

/// 为所有 content type 注册动态路由（启动时调用）
///
/// 除标准 CRUD 路由外，还为启用 `versioning` 的 content type 注册版本历史端点。
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

        if ct.versioning {
            let p = plural.clone();
            api = api
                .route(
                    &format!("/admin/cms/{p}/{{id}}/revisions"),
                    axum::routing::get(crate::handlers::content_revision::list_revisions),
                )
                .route(
                    &format!("/admin/cms/{p}/{{id}}/revisions/{{revision_id}}"),
                    axum::routing::get(crate::handlers::content_revision::get_revision),
                )
                .route(
                    &format!("/admin/cms/{p}/{{id}}/revisions/{{revision_id}}/restore"),
                    axum::routing::post(crate::handlers::content_revision::restore_revision),
                )
                .route(
                    &format!("/admin/cms/{p}/{{id}}/revisions/{{rev_a}}/diff/{{rev_b}}"),
                    axum::routing::get(crate::handlers::content_revision::diff_revisions),
                );
        }

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

    let save_ctx = SaveContext::from_optional_auth(&auth);

    match (method.clone(), id) {
        (axum::http::Method::GET, None) => {
            check_api_access(ct.api.list.access, auth.0.as_ref())?;
            let data = do_list(&state, &ct, params, auth.0.as_ref()).await?;
            Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
        }
        (axum::http::Method::POST, None) => {
            check_api_access(ct.api.create.access, auth.0.as_ref())?;
            let Json(data) = body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
            let result = do_create(&state, &ct, data, &save_ctx).await?;
            Ok((
                StatusCode::CREATED,
                Json(crate::errors::response::ApiResponse::success(result)),
            )
                .into_response())
        }
        (axum::http::Method::GET, Some(id)) => {
            check_api_access(ct.api.get.access, auth.0.as_ref())?;
            let data = do_get(&state, &ct, &id, auth.0.as_ref()).await?;
            Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
        }
        (axum::http::Method::PUT, Some(id)) => {
            check_api_access(ct.api.update.access, auth.0.as_ref())?;
            let Json(data) = body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
            let result = do_update(&state, &ct, &id, data, &save_ctx, auth.0.as_ref()).await?;
            Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
        }
        (axum::http::Method::DELETE, Some(id)) => {
            check_api_access(ct.api.delete.access, auth.0.as_ref())?;
            do_delete(&state, &ct, &id, auth.0.as_ref()).await?;
            Ok(Json(crate::errors::response::ApiResponse::success(
                json!({"deleted": true}),
            ))
            .into_response())
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
            Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
        }
        (axum::http::Method::GET, Some(id)) => {
            let data = do_admin_get(&state, &ct, &id).await?;
            Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
        }
        _ => Err(AppError::not_found(&format!("{method} {path}"))),
    }
}

// ── 核心业务逻辑（共享于固定路由和动态路由） ──────────────────────

pub fn cms_list_cache_key(ct: &ContentTypeSchema, query: &ContentQuery) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    query.status.hash(&mut hasher);
    query.page.hash(&mut hasher);
    query.page_size.hash(&mut hasher);
    query.search.hash(&mut hasher);
    query.sort.hash(&mut hasher);
    for (k, v) in &query.filters {
        k.hash(&mut hasher);
        v.to_string().hash(&mut hasher);
    }
    if let Some(ref inc) = query.include {
        for i in inc {
            i.hash(&mut hasher);
        }
    }
    let h = hasher.finish();
    format!("cms:{}:{h:x}", ct.plural)
}

pub fn cms_detail_cache_key(ct: &ContentTypeSchema, id_or_slug: &str) -> String {
    format!("cms:{}:detail:{id_or_slug}", ct.plural)
}

fn invalidate_cms_cache(state: &AppState, ct: &ContentTypeSchema) {
    let prefix = format!("cms:{}:", ct.plural);
    state.cms_cache.retain(|k, _| !k.starts_with(&prefix));
}

pub async fn do_list(
    state: &AppState,
    ct: &ContentTypeSchema,
    params: ListParams,
    auth: Option<&crate::middleware::auth::AuthIdentity>,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    let include = params.include.as_deref().map(parse_include);

    let (rule_where, rule_params) = ct
        .cached_rules
        .as_ref()
        .and_then(|r| build_rule_sql(&r.list, auth, &state.config.rule_engine))
        .map(|(w, p)| (Some(w), p))
        .unwrap_or_default();

    let filters: HashMap<String, Value> = params
        .extra
        .iter()
        .filter(|(key, _)| ct.get_field(key).is_some())
        .map(|(k, v)| {
            let col = ct
                .get_field(k)
                .and_then(|f| f.relation.as_ref().map(|r| r.foreign_key.clone()))
                .flatten()
                .unwrap_or_else(|| k.clone());
            (col, Value::String(v.clone()))
        })
        .collect();

    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters,
        status: if ct.draft_publish {
            Some(params.status.unwrap_or_else(|| "published".into()))
        } else {
            None
        },
        search: params.search,
        fields: ct.api.list.fields.clone(),
        tenant_id: None,
        include,
        skip_total: params.skip_total.unwrap_or(false),
        rule_where,
        rule_params,
        max_page_size: state.config.rule_engine.cms_max_page_size as i64,
        include_private: false,
    };

    let cache_key = cms_list_cache_key(ct, &query);
    let cache_ttl = std::time::Duration::from_secs(state.config.rule_engine.cms_cache_ttl_secs);
    if ct.api.list.cache
        && let Some(entry) = state.cms_cache.get(&cache_key)
        && entry.value().1.elapsed() < cache_ttl
    {
        return Ok(entry.value().0.clone());
    }

    let (items, total) = repo.find(ct, query.clone()).await?;
    let result = json!({
        "items": items,
        "total": total,
        "page": query.page,
        "page_size": query.page_size,
    });

    if ct.api.list.cache {
        state
            .cms_cache
            .insert(cache_key, (result.clone(), std::time::Instant::now()));
    }

    Ok(result)
}

pub async fn do_get(
    state: &AppState,
    ct: &ContentTypeSchema,
    id_or_slug: &str,
    auth: Option<&crate::middleware::auth::AuthIdentity>,
) -> Result<serde_json::Value, AppError> {
    let cache_key = cms_detail_cache_key(ct, id_or_slug);
    let cache_ttl = std::time::Duration::from_secs(state.config.rule_engine.cms_cache_ttl_secs);
    if ct.api.get.cache
        && let Some(entry) = state.cms_cache.get(&cache_key)
        && entry.value().1.elapsed() < cache_ttl
    {
        return Ok(entry.value().0.clone());
    }

    let repo = ContentRepository::new(state.pool.clone());
    let status = if ct.draft_publish {
        Some("published")
    } else {
        None
    };

    let item = if id_or_slug.contains('-') && !id_or_slug.contains('/') {
        repo.find_by_slug(ct, id_or_slug, status, None, false)
            .await?
            .or(None)
    } else {
        None
    };

    let item = match item {
        Some(data) => Some(data),
        None => repo.find_by_id(ct, id_or_slug, None, false).await?,
    };

    let result = item.ok_or_else(|| AppError::not_found(&format!("{}/{}", ct.name, id_or_slug)))?;

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.get.filter.as_ref()
    {
        let ctx = super::rule_engine::RuleContext::from_auth(auth);
        if !rule.evaluate(&result, &ctx, &state.config.rule_engine) {
            return Err(AppError::not_found(&format!("{}/{}", ct.name, id_or_slug)));
        }
    }

    state
        .plugins
        .dispatch_action(
            crate::plugins::HookPoint::ContentViewed,
            &json!({
                "content_type": ct.singular,
                "id": result.get("id").and_then(|v| v.as_str()).unwrap_or(""),
            }),
        )
        .await;

    if ct.api.get.cache {
        state
            .cms_cache
            .insert(cache_key, (result.clone(), std::time::Instant::now()));
    }

    let result = filter_fields(result, ct.api.get.fields.as_deref());

    Ok(result)
}

pub async fn do_create(
    state: &AppState,
    ct: &ContentTypeSchema,
    data: Value,
    save_ctx: &SaveContext,
) -> Result<serde_json::Value, AppError> {
    let hook_data = json!({
        "content_type": ct.singular,
        "data": &data,
    });
    let filtered = state
        .plugins
        .dispatch_filter(crate::plugins::HookPoint::ContentCreating, hook_data)
        .await?;

    let data = filtered.get("data").cloned().unwrap_or(data);

    let repo = ContentRepository::new(state.pool.clone());
    let result = repo.create(ct, data, None, save_ctx).await?;
    invalidate_cms_cache(state, ct);

    let id = result
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let slug = result
        .get("slug")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    state
        .plugins
        .dispatch_action(
            crate::plugins::HookPoint::ContentCreated,
            &json!({
                "content_type": ct.singular,
                "id": id,
                "slug": slug,
            }),
        )
        .await;

    Ok(result)
}

pub async fn do_update(
    state: &AppState,
    ct: &ContentTypeSchema,
    id: &str,
    data: Value,
    save_ctx: &SaveContext,
    auth: Option<&crate::middleware::auth::AuthIdentity>,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.update.filter.as_ref()
    {
        let existing = repo.find_by_id(ct, id, None, true).await?;
        if let Some(record) = existing {
            let ctx = super::rule_engine::RuleContext::from_auth(auth);
            if !rule.evaluate(&record, &ctx, &state.config.rule_engine) {
                return Err(AppError::Forbidden);
            }
        }
    }

    let hook_data = json!({
        "content_type": ct.singular,
        "id": id,
        "data": &data,
    });
    let filtered = state
        .plugins
        .dispatch_filter(crate::plugins::HookPoint::ContentUpdating, hook_data)
        .await?;

    let data = filtered.get("data").cloned().unwrap_or(data);

    let result = repo.update(ct, id, data, None, save_ctx).await?;
    invalidate_cms_cache(state, ct);

    state
        .plugins
        .dispatch_action(
            crate::plugins::HookPoint::ContentUpdated,
            &json!({
                "content_type": ct.singular,
                "id": id,
            }),
        )
        .await;

    Ok(result)
}

pub async fn do_delete(
    state: &AppState,
    ct: &ContentTypeSchema,
    id: &str,
    auth: Option<&crate::middleware::auth::AuthIdentity>,
) -> Result<(), AppError> {
    let repo = ContentRepository::new(state.pool.clone());

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.delete.filter.as_ref()
    {
        let existing = repo.find_by_id(ct, id, None, true).await?;
        if let Some(record) = existing {
            let ctx = super::rule_engine::RuleContext::from_auth(auth);
            if !rule.evaluate(&record, &ctx, &state.config.rule_engine) {
                return Err(AppError::Forbidden);
            }
        }
    }

    repo.delete(ct, id, None).await?;
    invalidate_cms_cache(state, ct);

    state
        .plugins
        .dispatch_action(
            crate::plugins::HookPoint::ContentDeleted,
            &json!({
                "content_type": ct.singular,
                "id": id,
            }),
        )
        .await;

    Ok(())
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
        skip_total: params.skip_total.unwrap_or(false),
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: state.config.rule_engine.cms_max_page_size as i64,
        include_private: true,
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
    let item = repo.find_by_id(ct, id, None, true).await?;
    item.ok_or_else(|| AppError::not_found(&format!("{}/{}", ct.name, id)))
}

// ── 固定路由 handler（启动时注册的 content type） ──────────────

async fn list_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    type_name: String,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.list.access, auth.0.as_ref())?;
    let data = do_list(&state, &ct, params, auth.0.as_ref()).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn get_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    Path(id_or_slug): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.get.access, auth.0.as_ref())?;
    let data = do_get(&state, &ct, &id_or_slug, auth.0.as_ref()).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn create_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    type_name: String,
    Json(data): Json<Value>,
) -> Result<
    (
        StatusCode,
        Json<crate::errors::response::ApiResponse<serde_json::Value>>,
    ),
    AppError,
> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.create.access, auth.0.as_ref())?;
    let save_ctx = SaveContext::from_optional_auth(&auth);
    let result = do_create(&state, &ct, data, &save_ctx).await?;
    Ok((
        StatusCode::CREATED,
        Json(crate::errors::response::ApiResponse::success(result)),
    ))
}

async fn update_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(data): Json<Value>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.update.access, auth.0.as_ref())?;
    let save_ctx = SaveContext::from_optional_auth(&auth);
    let result = do_update(&state, &ct, &id, data, &save_ctx, auth.0.as_ref()).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(result)))
}

async fn delete_handler(
    auth: OptionalAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.delete.access, auth.0.as_ref())?;
    do_delete(&state, &ct, &id, auth.0.as_ref()).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(
        json!({"deleted": true}),
    )))
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
    let data = do_admin_list(&state, &ct, params).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
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
    let data = do_admin_get(&state, &ct, &id).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
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
        name: req.name,
        singular: req.singular.clone(),
        plural: req.plural,
        table: req.table.clone(),
        description: req.description,
        draft_publish: req.draft_publish,
        slug_field: req.slug_field,
        timestamps: req.timestamps,
        soft_delete: req.soft_delete,
        versioning: req.versioning,
        fields: req.fields,
        indexes: vec![],
        list_view: None,
        api: super::schema::ApiConfig::default(),
        cached_column_names: None,
        cached_rules: None,
    };

    if crate::plugins::permissions::PermissionChecker::is_protected_table(
        &req.table,
        &state.config.protected_tables,
    ) {
        return Err(AppError::BadRequest(format!(
            "table '{}' is a protected system table",
            req.table
        )));
    }

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

    state
        .content_type_registry
        .register(schema.clone(), &state.config.rule_engine);

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

    let mut updated = (*ct).clone();

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
    if let Some(versioning) = req.versioning {
        updated.versioning = versioning;
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

    state
        .content_type_registry
        .register(updated.clone(), &state.config.rule_engine);

    tracing::info!(
        "updated content type: {} (table={}, hot-reload)",
        updated.singular,
        updated.table
    );

    Ok(Json(crate::errors::response::ApiResponse::success(
        updated.clone(),
    )))
}

/// 过滤 JSON 对象，只保留白名单字段 + 系统字段
/// 白名单为空时返回原始对象（不过滤）
fn filter_fields(mut value: serde_json::Value, fields: Option<&[String]>) -> serde_json::Value {
    let Some(allowed) = fields else {
        return value;
    };
    if allowed.is_empty() {
        return value;
    }
    let Some(obj) = value.as_object_mut() else {
        return value;
    };
    let system_keys: Vec<String> = obj
        .keys()
        .filter(|k| {
            *k == "id"
                || *k == "status"
                || *k == "published_at"
                || *k == "created_at"
                || *k == "updated_at"
                || *k == "deleted_at"
        })
        .cloned()
        .collect();
    obj.retain(|k, _| allowed.contains(&k.to_string()) || system_keys.contains(k));
    value
}

fn parse_include(s: &str) -> Vec<String> {
    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

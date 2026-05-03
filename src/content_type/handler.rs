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
use std::sync::Arc;

use super::repository::{ContentQuery, ContentRepository, SaveContext};
use super::rule_engine::compile_rule_sql;
use super::schema::{ContentKind, ContentTypeSchema, check_api_access};
use crate::AppState;
use crate::constants::*;
use crate::errors::app_error::AppError;
use crate::middleware::auth::AuthUser;

fn make_base_ctx_from_auth(
    auth: &AuthUser,
    pool: &crate::db::pool::Pool,
) -> crate::aspects::BaseContext {
    crate::aspects::BaseContext::new(
        auth.user_id().map(|s| s.to_string()),
        auth.tenant_id().unwrap_or(DEFAULT_TENANT).to_string(),
        crate::utils::tz::now_str(),
    )
    .with_pool(pool.clone())
}

fn make_base_ctx(state: &AppState, save_ctx: &SaveContext) -> crate::aspects::BaseContext {
    crate::aspects::BaseContext::new(
        save_ctx.user_id.clone(),
        save_ctx
            .tenant_id
            .clone()
            .unwrap_or_else(|| DEFAULT_TENANT.into()),
        crate::utils::tz::now_str(),
    )
    .with_pool(state.pool.clone())
}

fn make_base_ctx_anon(state: &AppState) -> crate::aspects::BaseContext {
    crate::aspects::BaseContext::new(None, DEFAULT_TENANT.into(), crate::utils::tz::now_str())
        .with_pool(state.pool.clone())
}

/// 编译 API Rule 为 SQL WHERE 子句。
///
/// 如果用户已登录且有 `filter_auth`，生成 `filter OR filter_auth`；
/// 否则只用 `filter`。
/// 如果规则需要认证但用户未登录，返回 None（调用方应拒绝请求）。
fn build_rule_sql(
    endpoint: &super::schema::CachedEndpointRules,
    auth: &AuthUser,
    config: &crate::config::app::RuleEngineConfig,
) -> Option<(String, Vec<String>)> {
    match (
        endpoint.filter.as_ref(),
        endpoint.filter_auth.as_ref(),
        auth.is_authenticated(),
    ) {
        (None, None, _) => None,
        (Some(rule), None, _) => compile_rule_sql(rule, 0, auth, config),
        (None, Some(_), false) => None,
        (Some(rule), Some(_), false) => compile_rule_sql(rule, 0, auth, config),
        (None, Some(auth_rule), true) => compile_rule_sql(auth_rule, 0, auth, config),
        (Some(rule), Some(auth_rule), true) => {
            let (base_sql, mut base_params) = compile_rule_sql(rule, 0, auth, config)?;
            let offset = base_params.len();
            let (auth_sql, mut auth_params) = compile_rule_sql(auth_rule, offset, auth, config)?;
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
    let cms = crate::constants::CMS_ROUTE;
    let admin_cms = crate::constants::CMS_ADMIN_ROUTE;

    for ct in registry.all() {
        let plural = ct.plural.clone();
        let singular = ct.singular.clone();

        if ct.kind == ContentKind::Single {
            api = api
                .route(
                    &format!("{cms}/{singular}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |auth, state| single_get_handler(auth, state, singular.clone())
                    })
                    .put({
                        let singular = singular.clone();
                        move |auth, state, data| {
                            single_update_handler(auth, state, data, singular.clone())
                        }
                    }),
                )
                .route(
                    &format!("{admin_cms}/{singular}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |state| admin_single_get_handler(state, singular.clone())
                    }),
                );
        } else {
            api = api
                .route(
                    &format!("{cms}/{plural}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |auth, state, params| {
                            list_handler(auth, state, singular.clone(), params)
                        }
                    })
                    .post({
                        let singular = singular.clone();
                        move |auth, state, data| create_handler(auth, state, singular.clone(), data)
                    }),
                )
                .route(
                    &format!("{cms}/{plural}/{{id_or_slug}}"),
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
                    &format!("{admin_cms}/{plural}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |state, params| admin_list_handler(state, singular.clone(), params)
                    }),
                )
                .route(
                    &format!("{admin_cms}/{plural}/{{id_or_slug}}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |state, path| admin_get_handler(state, path, singular.clone())
                    }),
                );
        }

        if ct.versioning {
            let p = plural.clone();
            api = api
                .route(
                    &format!("{admin_cms}/{p}/{{id}}/revisions"),
                    axum::routing::get(crate::handlers::content_revision::list_revisions),
                )
                .route(
                    &format!("{admin_cms}/{p}/{{id}}/revisions/{{revision_id}}"),
                    axum::routing::get(crate::handlers::content_revision::get_revision),
                )
                .route(
                    &format!("{admin_cms}/{p}/{{id}}/revisions/{{revision_id}}/restore"),
                    axum::routing::post(crate::handlers::content_revision::restore_revision),
                )
                .route(
                    &format!("{admin_cms}/{p}/{{id}}/revisions/{{rev_a}}/diff/{{rev_b}}"),
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
    let first = segments[0].to_string();
    let id = segments.get(1).map(|s| s.to_string());
    Some((first, id))
}

/// 根据 singular 或 plural 查找 content type
fn resolve_content_type(
    registry: &crate::content_type::ContentTypeRegistry,
    segment: &str,
) -> Option<(Arc<ContentTypeSchema>, bool)> {
    if let Some(ct) = registry.get(segment)
        && ct.is_single()
    {
        return Some((ct, true));
    }
    if let Some(ct) = registry.get_by_plural(segment) {
        return Some((ct, false));
    }
    None
}

/// Catch-all 动态路由 handler（启动后新增的 content type 走这里）
pub async fn dynamic_cms_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, AppError> {
    let Some((segment, id)) = parse_dynamic_path(&path) else {
        return Err(AppError::not_found("invalid cms path"));
    };

    let Some((ct, is_single)) = resolve_content_type(&state.content_type_registry, &segment) else {
        return Err(AppError::not_found(&segment));
    };

    let save_ctx = SaveContext::from_auth(&auth);

    if is_single {
        match (method.clone(), id) {
            (axum::http::Method::GET, None) => {
                check_api_access(ct.api.get.access, &auth)?;
                let data = do_single_get(&state, &ct, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::PUT, None) => {
                check_api_access(ct.api.update.access, &auth)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_single_update(&state, &ct, data, &save_ctx, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    } else {
        match (method.clone(), id) {
            (axum::http::Method::GET, None) => {
                check_api_access(ct.api.list.access, &auth)?;
                let data = do_list(&state, &ct, params, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::POST, None) => {
                check_api_access(ct.api.create.access, &auth)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_create(&state, &ct, data, &save_ctx).await?;
                Ok((
                    StatusCode::CREATED,
                    Json(crate::errors::response::ApiResponse::success(result)),
                )
                    .into_response())
            }
            (axum::http::Method::GET, Some(id)) => {
                check_api_access(ct.api.get.access, &auth)?;
                let data = do_get(&state, &ct, &id, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::PUT, Some(id)) => {
                check_api_access(ct.api.update.access, &auth)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_update(&state, &ct, &id, data, &save_ctx, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            (axum::http::Method::DELETE, Some(id)) => {
                check_api_access(ct.api.delete.access, &auth)?;
                do_delete(&state, &ct, &id, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(
                    json!({"deleted": true}),
                ))
                .into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    }
}

/// Catch-all admin 动态路由 handler
pub async fn dynamic_admin_cms_handler(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
    let Some((segment, id)) = parse_dynamic_path(&path) else {
        return Err(AppError::not_found("invalid admin cms path"));
    };

    let Some((ct, is_single)) = resolve_content_type(&state.content_type_registry, &segment) else {
        return Err(AppError::not_found(&segment));
    };

    if is_single {
        match method.clone() {
            axum::http::Method::GET => {
                let data = do_admin_single_get(&state, &ct).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    } else {
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
    auth: &AuthUser,
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

    {
        let mut read_ctx = crate::aspects::DataBeforeReadContext {
            base: make_base_ctx_anon(state),
            table: ct.table.clone(),
            query: crate::aspects::ReadQuery::default(),
            schema: Some(std::sync::Arc::new(ct.clone())),
        };
        if let Some(cached) = state
            .aspect_engine
            .dispatch_data_before_read(&ct.table, &mut read_ctx)
            .await
            .map_err(AppError::Internal)?
        {
            return Ok(cached);
        }
    }

    let (items, total) = repo.find(ct, query.clone()).await?;
    let mut items: Vec<Value> = items.into_iter().map(strip_meta).collect();

    {
        let records: Vec<crate::aspects::Record> = items
            .iter()
            .filter_map(|v| v.as_object().cloned())
            .collect();
        let mut after_ctx = crate::aspects::DataAfterReadContext {
            base: make_base_ctx_anon(state),
            table: ct.table.clone(),
            records,
            schema: Some(std::sync::Arc::new(ct.clone())),
        };
        if let Err(e) = state
            .aspect_engine
            .dispatch_data_after_read(&ct.table, &mut after_ctx)
            .await
        {
            tracing::warn!("aspect after_read dispatch error: {e}");
        } else {
            items = after_ctx.records.into_iter().map(Value::Object).collect();
        }
    }

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
    auth: &AuthUser,
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
    let result = strip_meta(result);

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

    let mut data = filtered.get("data").cloned().unwrap_or(data);

    {
        let record = data.as_object().cloned().unwrap_or_default();
        let mut ctx = crate::aspects::DataBeforeCreateContext {
            base: make_base_ctx(state, save_ctx),
            table: ct.table.clone(),
            record,
            schema: Some(std::sync::Arc::new(ct.clone())),
        };
        state
            .aspect_engine
            .dispatch_data_before_create(&ct.table, &mut ctx)
            .await
            .map_err(AppError::Internal)?;
        data = Value::Object(ctx.record);
    }

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

    {
        let record = result.as_object().cloned().unwrap_or_default();
        let mut after_ctx = crate::aspects::DataAfterCreateContext {
            base: make_base_ctx(state, save_ctx),
            table: ct.table.clone(),
            record,
            schema: Some(std::sync::Arc::new(ct.clone())),
        };
        if let Err(e) = state
            .aspect_engine
            .dispatch_data_after_create(&ct.table, &mut after_ctx)
            .await
        {
            tracing::warn!("aspect after_create dispatch error: {e}");
        }
    }

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
    auth: &AuthUser,
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

    let old_record_value = repo.find_by_id(ct, id, None, true).await?;

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.update.filter.as_ref()
        && let Some(ref record) = old_record_value
    {
        let ctx = super::rule_engine::RuleContext::from_auth(auth);
        if !rule.evaluate(record, &ctx, &state.config.rule_engine) {
            return Err(AppError::Forbidden);
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

    let mut data = filtered.get("data").cloned().unwrap_or(data);

    let old_record = old_record_value
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    {
        let new_record = data.as_object().cloned().unwrap_or_default();
        let mut ctx = crate::aspects::DataBeforeUpdateContext {
            base: make_base_ctx(state, save_ctx),
            table: ct.table.clone(),
            old_record: old_record.clone(),
            new_record,
            schema: Some(std::sync::Arc::new(ct.clone())),
        };
        state
            .aspect_engine
            .dispatch_data_before_update(&ct.table, &mut ctx)
            .await
            .map_err(AppError::Internal)?;
        data = Value::Object(ctx.new_record);
    }

    let result = repo.update(ct, id, data, None, save_ctx).await?;
    invalidate_cms_cache(state, ct);

    {
        let new_record = result.as_object().cloned().unwrap_or_default();
        let mut after_ctx = crate::aspects::DataAfterUpdateContext {
            base: make_base_ctx(state, save_ctx),
            table: ct.table.clone(),
            old_record,
            new_record,
            schema: Some(std::sync::Arc::new(ct.clone())),
        };
        if let Err(e) = state
            .aspect_engine
            .dispatch_data_after_update(&ct.table, &mut after_ctx)
            .await
        {
            tracing::warn!("aspect after_update dispatch error: {e}");
        }
    }

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
    auth: &AuthUser,
) -> Result<(), AppError> {
    let repo = ContentRepository::new(state.pool.clone());

    let existing = repo.find_by_id(ct, id, None, true).await?;
    let value = existing.ok_or_else(|| AppError::not_found(&ct.singular))?;

    let record: crate::aspects::Record = match value.as_object() {
        Some(map) => map.clone(),
        None => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "record is not an object"
            )));
        }
    };

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.delete.filter.as_ref()
    {
        let ctx = super::rule_engine::RuleContext::from_auth(auth);
        if !rule.evaluate(&value, &ctx, &state.config.rule_engine) {
            return Err(AppError::Forbidden);
        }
    }

    let mut before_ctx = crate::aspects::DataBeforeDeleteContext {
        base: make_base_ctx_from_auth(auth, &state.pool),
        table: ct.table.clone(),
        record: record.clone(),
        soft_delete: false,
        schema: Some(std::sync::Arc::new(ct.clone())),
    };
    state
        .aspect_engine
        .dispatch_data_before_delete(&ct.table, &mut before_ctx)
        .await
        .map_err(crate::errors::app_error::AppError::Internal)?;

    if before_ctx.soft_delete {
        let deleted_at = before_ctx
            .record
            .get(COL_DELETED_AT)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let deleted_by = before_ctx
            .record
            .get(COL_DELETED_BY)
            .and_then(|v| v.as_str());
        repo.soft_delete(ct, id, deleted_at, deleted_by, auth.tenant_id())
            .await?;
    } else {
        repo.delete(ct, id, auth.tenant_id()).await?;
    }

    invalidate_cms_cache(state, ct);

    {
        let mut after_ctx = crate::aspects::DataAfterDeleteContext {
            base: make_base_ctx_from_auth(auth, &state.pool),
            table: ct.table.clone(),
            record,
            schema: Some(std::sync::Arc::new(ct.clone())),
        };
        if let Err(e) = state
            .aspect_engine
            .dispatch_data_after_delete(&ct.table, &mut after_ctx)
            .await
        {
            tracing::warn!("aspect after_delete dispatch error: {e}");
        }
    }

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

pub async fn do_single_get(
    state: &AppState,
    ct: &ContentTypeSchema,
    auth: &AuthUser,
) -> Result<serde_json::Value, AppError> {
    let cache_key = format!("cms:{}:single", ct.singular);
    let cache_ttl = std::time::Duration::from_secs(state.config.rule_engine.cms_cache_ttl_secs);
    if ct.api.get.cache
        && let Some(entry) = state.cms_cache.get(&cache_key)
        && entry.value().1.elapsed() < cache_ttl
    {
        return Ok(entry.value().0.clone());
    }

    let repo = ContentRepository::new(state.pool.clone());
    let result = repo.ensure_single(ct, None).await?;

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.get.filter.as_ref()
    {
        let ctx = super::rule_engine::RuleContext::from_auth(auth);
        if !rule.evaluate(&result, &ctx, &state.config.rule_engine) {
            return Err(AppError::not_found(&ct.name));
        }
    }

    if ct.api.get.cache {
        state
            .cms_cache
            .insert(cache_key, (result.clone(), std::time::Instant::now()));
    }

    let result = filter_fields(result, ct.api.get.fields.as_deref());
    let result = strip_meta(result);
    Ok(result)
}

pub async fn do_single_update(
    state: &AppState,
    ct: &ContentTypeSchema,
    data: Value,
    save_ctx: &SaveContext,
    auth: &AuthUser,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    let existing = repo.ensure_single(ct, None).await?;
    let id = existing
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    do_update(state, ct, &id, data, save_ctx, auth).await
}

async fn do_admin_single_get(
    state: &AppState,
    ct: &ContentTypeSchema,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    repo.ensure_single(ct, None).await
}

// ── 固定路由 handler（启动时注册的 content type） ──────────────

async fn single_get_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.get.access, &auth)?;
    let data = do_single_get(&state, &ct, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn single_update_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(data): Json<Value>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.update.access, &auth)?;
    let save_ctx = SaveContext::from_auth(&auth);
    let result = do_single_update(&state, &ct, data, &save_ctx, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(result)))
}

async fn admin_single_get_handler(
    State(state): State<AppState>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let data = do_admin_single_get(&state, &ct).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn list_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    type_name: String,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.list.access, &auth)?;
    let data = do_list(&state, &ct, params, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn get_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id_or_slug): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.get.access, &auth)?;
    let data = do_get(&state, &ct, &id_or_slug, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn create_handler(
    auth: AuthUser,
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
    check_api_access(ct.api.create.access, &auth)?;
    let save_ctx = SaveContext::from_auth(&auth);
    let result = do_create(&state, &ct, data, &save_ctx).await?;
    Ok((
        StatusCode::CREATED,
        Json(crate::errors::response::ApiResponse::success(result)),
    ))
}

async fn update_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(data): Json<Value>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.update.access, &auth)?;
    let save_ctx = SaveContext::from_auth(&auth);
    let result = do_update(&state, &ct, &id, data, &save_ctx, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(result)))
}

async fn delete_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.delete.access, &auth)?;
    do_delete(&state, &ct, &id, &auth).await?;
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
        kind: req.kind,
        draft_publish: req.draft_publish,
        slug_field: req.slug_field,
        soft_delete: req.soft_delete,
        versioning: req.versioning,
        builtin: req.builtin,
        implements: req.implements,
        fields: req.fields,
        indexes: vec![],
        list_view: None,
        api: super::schema::ApiConfig::default(),
        cached_column_names: None,
        cached_rules: None,
    };

    if crate::plugins::permissions::PermissionChecker::is_protected_table(
        &schema.table,
        &state.config.builtins.protected_tables(),
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

    let reserved = state.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = state.protocol_registry.names();
    state
        .content_type_registry
        .register(
            schema.clone(),
            &state.config.rule_engine,
            &reserved,
            &protocol_names,
        )
        .map_err(|e| AppError::Conflict(e.to_string()))?;

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
    if let Some(soft_delete) = req.soft_delete {
        updated.soft_delete = soft_delete;
    }
    if let Some(versioning) = req.versioning {
        updated.versioning = versioning;
    }
    if let Some(implements) = req.implements {
        updated.implements = implements;
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

    let reserved = state.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = state.protocol_registry.names();
    state
        .content_type_registry
        .register(
            updated.clone(),
            &state.config.rule_engine,
            &reserved,
            &protocol_names,
        )
        .map_err(|e| AppError::Conflict(e.to_string()))?;

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

fn strip_meta(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object_mut() {
        obj.remove(COL_META);
    }
    value
}

fn parse_include(s: &str) -> Vec<String> {
    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

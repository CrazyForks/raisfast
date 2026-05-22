//! Generic content type API handler
//!
//! Automatically provides CRUD HTTP endpoints for all registered content types.
//! At startup, fixed routes are registered for known content types; content types added after
//! startup are handled via catch-all dynamic routes. Both route types share the same core logic.

use crate::types::snowflake_id::SnowflakeId;
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
use super::schema::{ContentKind, ContentTypeSchema, FieldType, RelationType, check_api_access};
use crate::AppState;
use crate::constants::*;
use crate::errors::app_error::AppError;
use crate::event::Event;
use crate::middleware::auth::AuthUser;

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
        "/admin/content-types",
        get,
        list_schemas,
        "system admin",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types",
        create,
        create_schema,
        "system admin",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types/{singular}",
        get,
        get_schema,
        "system admin",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types/{singular}",
        put,
        update_schema,
        "system admin",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types/{singular}",
        delete,
        delete_schema,
        "system admin",
        "admin/content-types"
    );
    let r = {
        let mr = axum::routing::any(dynamic_cms_handler);
        r.route("/cms/{*path}", mr)
    };
    registry.record("ANY", "/api/v1/cms/{*path}", "content_type public", "cms");
    let r = {
        let mr = axum::routing::any(dynamic_admin_cms_handler);
        r.route("/admin/cms/{*path}", mr)
    };
    registry.record(
        "ANY",
        "/api/v1/admin/cms/{*path}",
        "content_type admin",
        "admin/cms",
    );
    r
}

fn make_base_ctx_from_auth(
    auth: &AuthUser,
    pool: &crate::db::pool::Pool,
) -> crate::aspects::BaseContext {
    crate::aspects::BaseContext::new(
        auth.user_id().map(|id| id.to_string()),
        auth.tenant_id().unwrap_or(DEFAULT_TENANT).to_string(),
        crate::utils::tz::now_str(),
    )
    .with_pool(pool.clone())
    .with_user_int_id(auth.user_id())
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
    .with_user_int_id(save_ctx.user_int_id)
}

fn make_base_ctx_anon(state: &AppState) -> crate::aspects::BaseContext {
    crate::aspects::BaseContext::new(None, DEFAULT_TENANT.into(), crate::utils::tz::now_str())
        .with_pool(state.pool.clone())
}

/// Compile API Rules into SQL WHERE clauses.
///
/// If the user is authenticated and has `filter_auth`, generates `filter OR filter_auth`;
/// otherwise uses only `filter`.
/// Returns None if the rule requires authentication but the user is not logged in (caller should reject the request).
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

/// Pagination query parameters
#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub sort: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub include: Option<String>,
    /// Skip COUNT(*) query for performance, total returns -1
    #[serde(default)]
    pub skip_total: Option<bool>,
    /// Additional field filters (matching content type schema field names)
    #[serde(flatten)]
    pub extra: HashMap<String, String>,
}

/// Register dynamic routes for all content types (called at startup)
///
/// In addition to standard CRUD routes, also registers version history endpoints for content types with `versioning` enabled.
pub fn register_content_routes(
    router: axum::Router<AppState>,
    ct_registry: &crate::content_type::ContentTypeRegistry,
    protocol_registry: &crate::protocols::ProtocolRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<AppState> {
    let mut api = router;
    let restful = config.api_restful;
    let cms = crate::constants::CMS_ROUTE;
    let admin_cms = crate::constants::CMS_ADMIN_ROUTE;

    for ct in ct_registry.all() {
        let plural = ct.plural.clone();
        let singular = ct.singular.clone();

        if ct.kind == ContentKind::Single {
            if restful {
                api = api.route(
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
                );
            } else {
                api = api
                    .route(
                        &format!("{cms}/{singular}"),
                        axum::routing::get({
                            let singular = singular.clone();
                            move |auth, state| single_get_handler(auth, state, singular.clone())
                        }),
                    )
                    .route(
                        &format!("{cms}/{singular}/update"),
                        axum::routing::post({
                            let singular = singular.clone();
                            move |auth, state, data| {
                                single_update_handler(auth, state, data, singular.clone())
                            }
                        }),
                    );
            }
            api = api.route(
                &format!("{admin_cms}/{singular}"),
                axum::routing::get({
                    let singular = singular.clone();
                    move |state| admin_single_get_handler(state, singular.clone())
                }),
            );
        } else if restful {
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
                    &format!("{cms}/{plural}/{{id}}"),
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
                    &format!("{admin_cms}/{plural}/{{id}}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |state, path| admin_get_handler(state, path, singular.clone())
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
                    }),
                )
                .route(
                    &format!("{cms}/{plural}/create"),
                    axum::routing::post({
                        let singular = singular.clone();
                        move |auth, state, data| create_handler(auth, state, singular.clone(), data)
                    }),
                )
                .route(
                    &format!("{cms}/{plural}/{{id}}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |auth, state, path| get_handler(auth, state, path, singular.clone())
                    }),
                )
                .route(
                    &format!("{cms}/{plural}/{{id}}/update"),
                    axum::routing::post({
                        let singular = singular.clone();
                        move |auth, state, path, data| {
                            update_handler(auth, state, path, data, singular.clone())
                        }
                    }),
                )
                .route(
                    &format!("{cms}/{plural}/{{id}}/delete"),
                    axum::routing::post({
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
                    &format!("{admin_cms}/{plural}/{{id}}"),
                    axum::routing::get({
                        let singular = singular.clone();
                        move |state, path| admin_get_handler(state, path, singular.clone())
                    }),
                );
        }

        let protocol_names: Vec<String> =
            ct.implements.iter().map(|p| p.name().to_string()).collect();
        api = protocol_registry.register_routes_for(&protocol_names, api, &plural, admin_cms);

        tracing::debug!("registered CMS routes for content type: {}", ct.singular);
    }

    api
}

/// Parse catch-all path into (plural, optional id)
fn parse_dynamic_path(path: &str) -> Option<(String, Option<String>)> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    let first = segments[0].to_string();
    let id = segments.get(1).map(|s| s.to_string());
    Some((first, id))
}

/// Find content type by singular or plural name
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

/// Parse catch-all path into (segment, optional id, optional action)
fn parse_dynamic_path_with_action(path: &str) -> Option<(String, Option<String>, Option<String>)> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    let first = segments[0].to_string();
    match segments.len() {
        1 => Some((first, None, None)),
        2 => {
            let second = segments[1];
            match second {
                "create" | "update" | "delete" => Some((first, None, Some(second.to_string()))),
                _ => Some((first, Some(second.to_string()), None)),
            }
        }
        3 => {
            let action = segments[2];
            Some((
                first,
                Some(segments[1].to_string()),
                Some(action.to_string()),
            ))
        }
        _ => None,
    }
}

/// Catch-all dynamic route handler (for content types added after startup)
pub async fn dynamic_cms_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    body: Option<Json<Value>>,
) -> Result<axum::response::Response, AppError> {
    let restful = state.config.api_restful;

    if restful {
        dynamic_cms_dispatch_restful(auth, &state, method, &path, params, body).await
    } else {
        dynamic_cms_dispatch_simple(auth, &state, method, &path, params, body).await
    }
}

async fn dynamic_cms_dispatch_restful(
    auth: AuthUser,
    state: &AppState,
    method: axum::http::Method,
    path: &str,
    params: ListParams,
    body: Option<Json<Value>>,
) -> Result<axum::response::Response, AppError> {
    let Some((segment, id)) = parse_dynamic_path(path) else {
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
                let data = do_single_get(state, &ct, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::PUT, None) => {
                check_api_access(ct.api.update.access, &auth)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_single_update(state, &ct, data, &save_ctx, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    } else {
        match (method.clone(), id) {
            (axum::http::Method::GET, None) => {
                check_api_access(ct.api.list.access, &auth)?;
                let data = do_list(state, &ct, params, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::POST, None) => {
                check_api_access(ct.api.create.access, &auth)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_create(state, &ct, data, &save_ctx).await?;
                Ok((
                    StatusCode::CREATED,
                    Json(crate::errors::response::ApiResponse::success(result)),
                )
                    .into_response())
            }
            (axum::http::Method::GET, Some(id)) => {
                check_api_access(ct.api.get.access, &auth)?;
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let data = do_get(state, &ct, int_id, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::PUT, Some(id)) => {
                check_api_access(ct.api.update.access, &auth)?;
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_update(state, &ct, int_id, data, &save_ctx, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            (axum::http::Method::DELETE, Some(id)) => {
                check_api_access(ct.api.delete.access, &auth)?;
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                do_delete(state, &ct, int_id, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(
                    json!({"deleted": true}),
                ))
                .into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    }
}

async fn dynamic_cms_dispatch_simple(
    auth: AuthUser,
    state: &AppState,
    method: axum::http::Method,
    path: &str,
    params: ListParams,
    body: Option<Json<Value>>,
) -> Result<axum::response::Response, AppError> {
    let Some((segment, id, action)) = parse_dynamic_path_with_action(path) else {
        return Err(AppError::not_found("invalid cms path"));
    };

    let Some((ct, is_single)) = resolve_content_type(&state.content_type_registry, &segment) else {
        return Err(AppError::not_found(&segment));
    };

    let save_ctx = SaveContext::from_auth(&auth);

    if is_single {
        match (method.clone(), id, action.as_deref()) {
            (axum::http::Method::GET, None, None) => {
                check_api_access(ct.api.get.access, &auth)?;
                let data = do_single_get(state, &ct, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::POST, None, Some("update")) => {
                check_api_access(ct.api.update.access, &auth)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_single_update(state, &ct, data, &save_ctx, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    } else {
        match (method.clone(), id, action.as_deref()) {
            (axum::http::Method::GET, None, None) => {
                check_api_access(ct.api.list.access, &auth)?;
                let data = do_list(state, &ct, params, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::POST, None, Some("create")) => {
                check_api_access(ct.api.create.access, &auth)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_create(state, &ct, data, &save_ctx).await?;
                Ok((
                    StatusCode::CREATED,
                    Json(crate::errors::response::ApiResponse::success(result)),
                )
                    .into_response())
            }
            (axum::http::Method::GET, Some(id), None) => {
                check_api_access(ct.api.get.access, &auth)?;
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let data = do_get(state, &ct, int_id, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::POST, Some(id), Some("update")) => {
                check_api_access(ct.api.update.access, &auth)?;
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let Json(data) =
                    body.ok_or_else(|| AppError::BadRequest("body required".into()))?;
                let result = do_update(state, &ct, int_id, data, &save_ctx, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            (axum::http::Method::POST, Some(id), Some("delete")) => {
                check_api_access(ct.api.delete.access, &auth)?;
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                do_delete(state, &ct, int_id, &auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(
                    json!({"deleted": true}),
                ))
                .into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    }
}

/// Catch-all admin dynamic route handler
pub async fn dynamic_admin_cms_handler(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<axum::response::Response, AppError> {
    let restful = state.config.api_restful;

    if restful {
        dynamic_admin_cms_dispatch_restful(&state, method, &path, params).await
    } else {
        dynamic_admin_cms_dispatch_simple(&state, method, &path, params).await
    }
}

async fn dynamic_admin_cms_dispatch_restful(
    state: &AppState,
    method: axum::http::Method,
    path: &str,
    params: ListParams,
) -> Result<axum::response::Response, AppError> {
    let Some((segment, id)) = parse_dynamic_path(path) else {
        return Err(AppError::not_found("invalid admin cms path"));
    };

    let Some((ct, is_single)) = resolve_content_type(&state.content_type_registry, &segment) else {
        return Err(AppError::not_found(&segment));
    };

    if is_single {
        match method.clone() {
            axum::http::Method::GET => {
                let data = do_admin_single_get(state, &ct).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    } else {
        match (method.clone(), id) {
            (axum::http::Method::GET, None) => {
                let data = do_admin_list(state, &ct, params).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::GET, Some(id)) => {
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let data = do_admin_get(state, &ct, int_id).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    }
}

async fn dynamic_admin_cms_dispatch_simple(
    state: &AppState,
    method: axum::http::Method,
    path: &str,
    params: ListParams,
) -> Result<axum::response::Response, AppError> {
    let Some((segment, id, action)) = parse_dynamic_path_with_action(path) else {
        return Err(AppError::not_found("invalid admin cms path"));
    };

    let Some((ct, is_single)) = resolve_content_type(&state.content_type_registry, &segment) else {
        return Err(AppError::not_found(&segment));
    };

    if action.is_some() {
        return Err(AppError::not_found(&format!("{method} {path}")));
    }

    if is_single {
        match method.clone() {
            axum::http::Method::GET => {
                let data = do_admin_single_get(state, &ct).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    } else {
        match (method.clone(), id) {
            (axum::http::Method::GET, None) => {
                let data = do_admin_list(state, &ct, params).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::GET, Some(id)) => {
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let data = do_admin_get(state, &ct, int_id).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    }
}

// ── Core business logic (shared between fixed and dynamic routes) ──────────────────────

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

pub fn cms_detail_cache_key(ct: &ContentTypeSchema, id: SnowflakeId) -> String {
    format!("cms:{}:detail:{id}", ct.plural)
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

    let mut meta_filters: Vec<(String, String)> = Vec::new();
    let meta_prefix = format!("{COL_META}.");
    let mut filters: HashMap<String, Value> = HashMap::new();

    for (key, v) in &params.extra {
        if let Some(path) = key.strip_prefix(&meta_prefix) {
            meta_filters.push((path.to_string(), v.clone()));
            continue;
        }
        let Some(field) = ct.get_field(key) else {
            continue;
        };

        if field.field_type == FieldType::Relation {
            if let Some(ref rel) = field.relation {
                match rel.relation_type {
                    RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay => {
                        let fk_col = rel
                            .foreign_key
                            .clone()
                            .unwrap_or_else(|| format!("{}_id", field.name));
                        let parsed_id =
                            crate::types::snowflake_id::parse_id(v).unwrap_or(SnowflakeId(-1));
                        let int_id =
                            raisfast_derive::crud_resolve_id!(&state.pool, &rel.target, *parsed_id)
                                .ok()
                                .flatten()
                                .unwrap_or(-1);
                        filters.insert(fk_col, json!(int_id));
                    }
                    _ => {}
                }
            }
        } else {
            filters.insert(key.clone(), Value::String(v.clone()));
        }
    }

    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters,
        status: None,
        search: params.search,
        fields: ct.api.list.fields.clone(),
        tenant_id: None,
        include,
        skip_total: params.skip_total.unwrap_or(false),
        rule_where,
        rule_params,
        max_page_size: state.config.rule_engine.cms_max_page_size as i64,
        include_private: false,
        meta_filters,
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
    let mut items: Vec<Value> = items;

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
    id: SnowflakeId,
    auth: &AuthUser,
) -> Result<serde_json::Value, AppError> {
    let cache_key = cms_detail_cache_key(ct, id);
    let cache_ttl = std::time::Duration::from_secs(state.config.rule_engine.cms_cache_ttl_secs);
    if ct.api.get.cache
        && let Some(entry) = state.cms_cache.get(&cache_key)
        && entry.value().1.elapsed() < cache_ttl
    {
        return Ok(entry.value().0.clone());
    }

    let repo = ContentRepository::new(state.pool.clone());
    let item = repo.find_by_id(ct, id, None, false).await?;
    let result = item.ok_or_else(|| AppError::not_found(&format!("{}/{}", ct.name, id)))?;

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.get.filter.as_ref()
    {
        let ctx = super::rule_engine::RuleContext::from_auth(auth);
        if !rule.evaluate(&result, &ctx, &state.config.rule_engine) {
            return Err(AppError::not_found(&format!("{}/{}", ct.name, id)));
        }
    }

    state
        .plugins
        .dispatch_action(
            "on_content_viewed",
            &json!({
                "content_type": ct.singular,
                "id": result.get(COL_ID).and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0),
            }),
        )
        .await;

    if ct.api.get.cache {
        state
            .cms_cache
            .insert(cache_key, (result.clone(), std::time::Instant::now()));
    }

    let result = filter_fields(result, ct.api.get.fields.as_deref(), ct);

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
        .dispatch_filter(&Event::ContentCreating, hook_data)
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
        .get(COL_ID)
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
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
            "on_content_created",
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
    id: SnowflakeId,
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
        .dispatch_filter(&Event::ContentUpdating, hook_data)
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
            "on_content_updated",
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
    id: SnowflakeId,
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
            .and_then(|v| v.as_i64());
        repo.soft_delete(ct, id, deleted_at, deleted_by, auth.tenant_id())
            .await?;
    } else {
        repo.delete(
            ct,
            id,
            auth.tenant_id(),
            &state.protocol_registry,
            &state.content_type_registry,
        )
        .await?;
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
            "on_content_deleted",
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

    let mut meta_filters: Vec<(String, String)> = Vec::new();
    let meta_prefix = format!("{COL_META}.");
    let mut filters: HashMap<String, Value> = HashMap::new();

    for (key, v) in &params.extra {
        if let Some(path) = key.strip_prefix(&meta_prefix) {
            meta_filters.push((path.to_string(), v.clone()));
            continue;
        }
        let Some(field) = ct.get_field(key) else {
            continue;
        };

        if field.field_type == FieldType::Relation {
            if let Some(ref rel) = field.relation {
                match rel.relation_type {
                    RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay => {
                        let fk_col = rel
                            .foreign_key
                            .clone()
                            .unwrap_or_else(|| format!("{}_id", field.name));
                        let parsed_id =
                            crate::types::snowflake_id::parse_id(v).unwrap_or(SnowflakeId(-1));
                        let int_id =
                            raisfast_derive::crud_resolve_id!(&state.pool, &rel.target, *parsed_id)
                                .ok()
                                .flatten()
                                .unwrap_or(-1);
                        filters.insert(fk_col, json!(int_id));
                    }
                    _ => {}
                }
            }
        } else {
            filters.insert(key.clone(), Value::String(v.clone()));
        }
    }

    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters,
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
        meta_filters,
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
    id: SnowflakeId,
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

    let result = filter_fields(result, ct.api.get.fields.as_deref(), ct);
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
    let id = existing.get(COL_ID).and_then(|v| v.as_i64()).unwrap_or(0);

    do_update(state, ct, SnowflakeId(id), data, save_ctx, auth).await
}

async fn do_admin_single_get(
    state: &AppState,
    ct: &ContentTypeSchema,
) -> Result<serde_json::Value, AppError> {
    let repo = ContentRepository::new(state.pool.clone());
    repo.ensure_single(ct, None).await
}

// ── Fixed route handlers (for content types registered at startup) ──────────────

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
    Path(id): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    check_api_access(ct.api.get.access, &auth)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let data = do_get(&state, &ct, id, &auth).await?;
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
    let int_id = crate::types::snowflake_id::parse_id(&id)?;
    let save_ctx = SaveContext::from_auth(&auth);
    let result = do_update(&state, &ct, int_id, data, &save_ctx, &auth).await?;
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
    let int_id = crate::types::snowflake_id::parse_id(&id)?;
    do_delete(&state, &ct, int_id, &auth).await?;
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
    let int_id = crate::types::snowflake_id::parse_id(&id)?;
    let data = do_admin_get(&state, &ct, int_id).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

// ── Schema Management API ──────────────────────────────────────────

/// GET /admin/content-types — List schema definitions of all registered content types
pub async fn list_schemas(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let schemas = state.content_type_registry.all();
    Ok(Json(crate::errors::response::ApiResponse::success(schemas)))
}

/// GET /admin/content-types/:singular — Get the schema definition of a single content type
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

/// POST /admin/content-types — Create a new content type
///
/// 1. Validate singular uniqueness
/// 2. Write TOML file
/// 3. Execute DB migration (create table / add columns)
/// 4. Register in memory ContentTypeRegistry (takes effect immediately, no restart required)
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
        slug_field: req.slug_field,
        builtin: req.builtin,
        implements: req.implements,
        fields: req.fields,
        indexes: vec![],
        api: super::schema::ApiConfig::default(),
        cached_column_names: None,
        cached_protocol_column_names: None,
        cached_behaviors: None,
        cached_declaration: None,
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
    repo.migrate(&schema, &state.protocol_registry).await?;

    let reserved = state.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = state.protocol_registry.names();
    state
        .content_type_registry
        .register(
            schema.clone(),
            &state.config.rule_engine,
            &reserved,
            &protocol_names,
            &state.protocol_registry,
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

/// DELETE /admin/content-types/:singular — Delete a content type
///
/// Deletes the TOML file and unregisters from the in-memory registry. Does not drop the database table.
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

/// PUT /admin/content-types/:singular — Update a content type schema
///
/// Incremental update: only modifies fields provided in the request. If `fields` is provided,
/// it is compared against the database and automatically `ALTER TABLE ADD COLUMN` to add missing
/// columns (does not delete columns or change column types).
/// The updated schema is synced to the in-memory registry (takes effect immediately, no restart required).
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
    if let Some(slug_field) = req.slug_field {
        updated.slug_field = slug_field;
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

    let dir = std::path::Path::new(&state.config.content_type_dir);
    updated.save_to_dir(dir)?;

    let repo = ContentRepository::new(state.pool.clone());
    repo.migrate(&updated, &state.protocol_registry).await?;

    let reserved = state.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = state.protocol_registry.names();
    state
        .content_type_registry
        .register(
            updated.clone(),
            &state.config.rule_engine,
            &reserved,
            &protocol_names,
            &state.protocol_registry,
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

/// Filter a JSON object, keeping only whitelisted fields + system fields
/// Returns the original object when whitelist is empty (no filtering)
fn filter_fields(
    mut value: serde_json::Value,
    fields: Option<&[String]>,
    ct: &super::schema::ContentTypeSchema,
) -> serde_json::Value {
    let Some(allowed) = fields else {
        return value;
    };
    if allowed.is_empty() {
        return value;
    }
    let Some(obj) = value.as_object_mut() else {
        return value;
    };
    let protocol_cols: Vec<&str> = ct.protocol_column_names();
    let system_keys: Vec<String> = obj
        .keys()
        .filter(|k| *k == COL_ID || protocol_cols.contains(&k.as_str()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_type::schema::ContentTypeSchema;

    fn parse_ct() -> ContentTypeSchema {
        ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"

[fields.title]
type = "text"
required = true

[fields.price]
type = "integer"
private = true

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
"#,
        )
        .unwrap()
    }

    #[test]
    fn parse_dynamic_path_empty() {
        assert!(parse_dynamic_path("").is_none());
    }

    #[test]
    fn parse_dynamic_path_plural_only() {
        let (seg, id) = parse_dynamic_path("products").unwrap();
        assert_eq!(seg, "products");
        assert!(id.is_none());
    }

    #[test]
    fn parse_dynamic_path_with_id() {
        let (seg, id) = parse_dynamic_path("products/abc-123").unwrap();
        assert_eq!(seg, "products");
        assert_eq!(id, Some("abc-123".to_string()));
    }

    #[test]
    fn parse_dynamic_path_leading_slash() {
        let (seg, id) = parse_dynamic_path("/products/xyz").unwrap();
        assert_eq!(seg, "products");
        assert_eq!(id, Some("xyz".to_string()));
    }

    #[test]
    fn parse_dynamic_path_trailing_slash() {
        let (seg, id) = parse_dynamic_path("products/").unwrap();
        assert_eq!(seg, "products");
        assert!(id.is_none());
    }

    #[test]
    fn parse_dynamic_path_with_action_plural_create() {
        let (seg, id, action) = parse_dynamic_path_with_action("products/create").unwrap();
        assert_eq!(seg, "products");
        assert!(id.is_none());
        assert_eq!(action, Some("create".to_string()));
    }

    #[test]
    fn parse_dynamic_path_with_action_id_update() {
        let (seg, id, action) = parse_dynamic_path_with_action("products/abc-123/update").unwrap();
        assert_eq!(seg, "products");
        assert_eq!(id, Some("abc-123".to_string()));
        assert_eq!(action, Some("update".to_string()));
    }

    #[test]
    fn parse_dynamic_path_with_action_id_delete() {
        let (seg, id, action) = parse_dynamic_path_with_action("products/abc-123/delete").unwrap();
        assert_eq!(seg, "products");
        assert_eq!(id, Some("abc-123".to_string()));
        assert_eq!(action, Some("delete".to_string()));
    }

    #[test]
    fn parse_dynamic_path_with_action_no_action() {
        let (seg, id, action) = parse_dynamic_path_with_action("products/abc-123").unwrap();
        assert_eq!(seg, "products");
        assert_eq!(id, Some("abc-123".to_string()));
        assert!(action.is_none());
    }

    #[test]
    fn parse_dynamic_path_with_action_empty() {
        assert!(parse_dynamic_path_with_action("").is_none());
    }

    #[test]
    fn parse_dynamic_path_with_action_update_no_id() {
        let (seg, id, action) = parse_dynamic_path_with_action("setting/update").unwrap();
        assert_eq!(seg, "setting");
        assert!(id.is_none());
        assert_eq!(action, Some("update".to_string()));
    }

    #[test]
    fn resolve_content_type_by_singular_single() {
        let registry = crate::content_type::ContentTypeRegistry::new();
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Setting"
singular = "setting"
plural = "settings"
table = "settings"
kind = "single"

[fields.key]
type = "text"
"#,
        )
        .unwrap();
        let reg = crate::protocols::ProtocolRegistry::new();
        let _ = registry.register(ct, &Default::default(), &[], &[], &reg);
        let (found, is_single) = resolve_content_type(&registry, "setting").unwrap();
        assert!(is_single);
        assert_eq!(found.singular, "setting");
    }

    #[test]
    fn resolve_content_type_by_plural() {
        let registry = crate::content_type::ContentTypeRegistry::new();
        let ct = parse_ct();
        let reg = crate::protocols::ProtocolRegistry::new();
        let _ = registry.register(ct, &Default::default(), &[], &[], &reg);
        let (found, is_single) = resolve_content_type(&registry, "products").unwrap();
        assert!(!is_single);
        assert_eq!(found.singular, "product");
    }

    #[test]
    fn resolve_content_type_not_found() {
        let registry = crate::content_type::ContentTypeRegistry::new();
        assert!(resolve_content_type(&registry, "nothing").is_none());
    }

    #[test]
    fn filter_fields_with_whitelist() {
        let ct = parse_ct();
        let data = json!({"title": "Hello", "price": 100, "id": 1, "extra": "x"});
        let filtered = filter_fields(data, Some(&["title".to_string()]), &ct);
        let obj = filtered.as_object().unwrap();
        assert!(obj.contains_key("title"));
        assert!(obj.contains_key("id"));
        assert!(!obj.contains_key("price"));
        assert!(!obj.contains_key("extra"));
    }

    #[test]
    fn filter_fields_no_whitelist() {
        let ct = parse_ct();
        let data = json!({"title": "Hello", "price": 100});
        let filtered = filter_fields(data, None, &ct);
        assert_eq!(filtered["title"], "Hello");
        assert_eq!(filtered["price"], 100);
    }

    #[test]
    fn filter_fields_empty_whitelist() {
        let ct = parse_ct();
        let data = json!({"title": "Hello"});
        let filtered = filter_fields(data, Some(&[]), &ct);
        assert_eq!(filtered["title"], "Hello");
    }

    #[test]
    fn filter_fields_non_object_passthrough() {
        let ct = parse_ct();
        let data = json!("string");
        let filtered = filter_fields(data, Some(&["title".to_string()]), &ct);
        assert_eq!(filtered, json!("string"));
    }

    #[test]
    fn parse_include_basic() {
        let result = parse_include("author,tags,comments");
        assert_eq!(result, vec!["author", "tags", "comments"]);
    }

    #[test]
    fn parse_include_with_spaces() {
        let result = parse_include(" author , tags ");
        assert_eq!(result, vec!["author", "tags"]);
    }

    #[test]
    fn parse_include_empty() {
        let result = parse_include("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_include_trailing_comma() {
        let result = parse_include("a,b,");
        assert_eq!(result, vec!["a", "b"]);
    }

    #[test]
    fn cms_detail_cache_key_format() {
        let ct = parse_ct();
        let key = cms_detail_cache_key(&ct, SnowflakeId(123));
        assert_eq!(key, "cms:products:detail:123");
    }

    #[test]
    fn cms_list_cache_key_contains_plural() {
        let ct = parse_ct();
        let query = crate::content_type::repository::ContentQuery {
            page: 1,
            page_size: 20,
            sort: None,
            filters: Default::default(),
            status: None,
            search: None,
            fields: None,
            tenant_id: None,
            include: None,
            rule_where: None,
            rule_params: Vec::new(),
            meta_filters: Vec::new(),
            skip_total: false,
            max_page_size: 100,
            include_private: false,
        };
        let key = cms_list_cache_key(&ct, &query);
        assert!(key.starts_with("cms:products:"));
    }

    #[test]
    fn cms_list_cache_key_differs_for_different_params() {
        let ct = parse_ct();
        let q1 = crate::content_type::repository::ContentQuery {
            page: 1,
            page_size: 20,
            sort: None,
            filters: Default::default(),
            status: None,
            search: None,
            fields: None,
            tenant_id: None,
            include: None,
            rule_where: None,
            rule_params: Vec::new(),
            meta_filters: Vec::new(),
            skip_total: false,
            max_page_size: 100,
            include_private: false,
        };
        let q2 = crate::content_type::repository::ContentQuery {
            page: 2,
            ..q1.clone()
        };
        let k1 = cms_list_cache_key(&ct, &q1);
        let k2 = cms_list_cache_key(&ct, &q2);
        assert_ne!(k1, k2);
    }
}

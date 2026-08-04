//! Generic content type API handler
//!
//! Automatically provides CRUD HTTP endpoints for all registered content types.
//! At startup, fixed routes are registered for known content types; content types added after
//! startup are handled via catch-all dynamic routes. Both route types share the same core logic.

use crate::types::snowflake_id::SnowflakeId;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use super::repository::{
    ContentQuery, ContentRepository, FieldFilter, FilterOp, MetaFilter, SaveContext,
};
use super::rule_engine::{Rule, compile_rule_sql};
use super::schema::{
    ApiAccess, ContentKind, ContentTypeSchema, FieldType, RelationType, check_api_access,
};
use crate::AppState;
use crate::constants::*;
use crate::db::DbDriver;
use crate::dto::batch::{BatchRequest, BatchResponse, BulkImportResponse};
use crate::errors::app_error::AppError;
use crate::errors::validation::validate;
use crate::event::Event;
use crate::middleware::auth::{AuthUser, TokenAction};

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
        "content type",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types",
        create,
        create_schema,
        "content type",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types/{*ct_path}",
        get,
        get_schema,
        "content type",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types/{*ct_path}",
        put,
        update_schema,
        "content type",
        "admin/content-types"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/content-types/{*ct_path}",
        delete,
        delete_schema,
        "content type",
        "admin/content-types"
    );
    let r = {
        let mr = axum::routing::any(dynamic_cms_handler);
        r.route("/cms/{*path}", mr)
    };
    registry.record("ANY", "/api/v1/cms/{*path}", "content type", "cms");
    let r = {
        let mr = axum::routing::any(dynamic_admin_cms_handler);
        r.route("/admin/cms/{*path}", mr)
    };
    registry.record(
        "ANY",
        "/api/v1/admin/cms/{*path}",
        "content type",
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
/// Check if the authenticated user is the creator of the given record.
fn is_owner(record: &serde_json::Value, auth: &AuthUser) -> bool {
    let Some(uid) = auth.user_id() else {
        return false;
    };
    match record.get(COL_CREATED_BY) {
        Some(serde_json::Value::Number(n)) => n.as_i64().is_some_and(|v| v == uid),
        Some(serde_json::Value::String(s)) => s.parse::<i64>().is_ok_and(|v| v == uid),
        _ => false,
    }
}

/// Parse a query key like `price[$gt]` into `("price", FilterOp::Gt)`.
///
/// Plain field names (no bracket suffix) fall back to equality via the caller.
fn parse_op_key(key: &str) -> Option<(&str, FilterOp)> {
    let (field, rest) = key.split_once("[$")?;
    let op_name = rest.strip_suffix(']')?;
    Some((field, FilterOp::from_suffix(&format!("${op_name}"))?))
}

/// Build field filters from raw query parameters.
///
/// Supports PocketBase-style operator keys (`price[$gt]=100`) plus legacy
/// equality (`price=100`). Relation fields are resolved to FK columns.
/// The `status` query parameter is folded in as an equality filter when the
/// content type actually has a `status` column.
async fn parse_field_filters(
    ct: &ContentTypeSchema,
    state: &AppState,
    extra: &HashMap<String, String>,
    status: Option<String>,
) -> Vec<FieldFilter> {
    let mut filters: Vec<FieldFilter> = Vec::new();
    for (key, v) in extra {
        if key.starts_with(&format!("{COL_META}.")) {
            continue;
        }
        let (field, op) = parse_op_key(key).unwrap_or((key.as_str(), FilterOp::Eq));
        let Some(field_schema) = ct.get_field(field) else {
            if ct.has_column(field) {
                filters.push(FieldFilter {
                    field: field.to_string(),
                    op,
                    value: Value::String(v.clone()),
                });
            }
            continue;
        };

        if field_schema.field_type == FieldType::Relation {
            let resolved = resolve_relation_filter(state, field_schema, op, v).await;
            filters.push(match resolved {
                Some((fk_col, value)) => FieldFilter {
                    field: fk_col,
                    op,
                    value,
                },
                None => FieldFilter {
                    field: field.to_string(),
                    op,
                    value: Value::String(v.clone()),
                },
            });
        } else {
            filters.push(FieldFilter {
                field: field.to_string(),
                op,
                value: Value::String(v.clone()),
            });
        }
    }

    if let Some(ref status) = status
        && ct.has_column(COL_STATUS)
    {
        filters.push(FieldFilter {
            field: COL_STATUS.into(),
            op: FilterOp::Eq,
            value: Value::String(status.clone()),
        });
    }
    filters
}

/// Resolve a relation field filter to its FK column + numeric id value.
///
/// For `$in` / `$nin` the comma-separated slugs are each resolved. Returns
/// `None` for relation types that do not store a local FK column.
async fn resolve_relation_filter(
    state: &AppState,
    field_schema: &super::schema::FieldSchema,
    op: FilterOp,
    v: &str,
) -> Option<(String, Value)> {
    let rel = field_schema.relation.as_ref()?;
    if !matches!(
        rel.relation_type,
        RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay
    ) {
        return None;
    }
    let fk_col = rel
        .foreign_key
        .clone()
        .unwrap_or_else(|| format!("{}_id", field_schema.name));
    if op == FilterOp::In || op == FilterOp::Nin {
        let mut ids: Vec<String> = Vec::new();
        for part in v.split(',') {
            let parsed =
                crate::types::snowflake_id::parse_id(part.trim()).unwrap_or(SnowflakeId(-1));
            if let Some(id) = raisfast_derive::crud_resolve_id!(&state.pool, &rel.target, *parsed)
                .ok()
                .flatten()
            {
                ids.push(id.to_string());
            }
        }
        Some((fk_col, json!(ids.join(","))))
    } else {
        let parsed_id = crate::types::snowflake_id::parse_id(v).unwrap_or(SnowflakeId(-1));
        let int_id = raisfast_derive::crud_resolve_id!(&state.pool, &rel.target, *parsed_id)
            .ok()
            .flatten()
            .unwrap_or(-1);
        Some((fk_col, json!(int_id)))
    }
}

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

/// Compile a user-supplied `?filter=` expression into SQL.
///
/// Reuses the API Rule engine. Returns `None` when the filter is empty,
/// malformed, or references `@request.auth` while unauthenticated.
fn compile_query_filter(
    expr: Option<&str>,
    offset: usize,
    auth: &AuthUser,
    config: &crate::config::app::RuleEngineConfig,
) -> Option<(String, Vec<String>)> {
    let expr = expr?.trim();
    if expr.is_empty() {
        return None;
    }
    let rule = Rule::parse(expr, config).ok()?;
    compile_rule_sql(&rule, offset, auth, config)
}

/// Merge the endpoint's cached API Rule (filter/filter_auth) with the
/// user-supplied `?filter=` expression. Both compile to SQL and are AND-ed.
fn compile_list_filters(
    cached: Option<(String, Vec<String>)>,
    filter: Option<&str>,
    auth: &AuthUser,
    config: &crate::config::app::RuleEngineConfig,
) -> (Option<String>, Vec<String>) {
    let (cached_where, mut all_params) = cached
        .map(|(w, p)| (Some(w), p))
        .unwrap_or((None, Vec::new()));

    let user_sql = compile_query_filter(filter, all_params.len(), auth, config);

    match (cached_where, user_sql) {
        (Some(w1), Some((w2, p2))) => {
            all_params.extend(p2);
            (Some(format!("({w1}) AND ({w2})")), all_params)
        }
        (Some(w1), None) => (Some(w1), all_params),
        (None, Some((w2, p2))) => (Some(w2), p2),
        (None, None) => (None, all_params),
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
    /// PocketBase-style filter expression, e.g. `price>=100&&price<=500`
    /// or `status="published"||author_id=123`. Reuses the API Rule engine
    /// and compiles to a SQL WHERE clause.
    pub filter: Option<String>,
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
        let group = ct.group.clone();
        // Registry key passed to handler closures for lookup (e.g. "poll" or "forum/poll")
        let registry_key = ct.registry_key();

        // Route prefix: "/cms/{group}/{plural}" when grouped, "/cms/{plural}" when flat
        let cms_prefix = if group.is_empty() {
            format!("{cms}/{plural}")
        } else {
            format!("{cms}/{group}/{plural}")
        };
        let admin_prefix = if group.is_empty() {
            format!("{admin_cms}/{plural}")
        } else {
            format!("{admin_cms}/{group}/{plural}")
        };

        if ct.kind == ContentKind::Single {
            let cms_single = if group.is_empty() {
                format!("{cms}/{singular}")
            } else {
                format!("{cms}/{group}/{singular}")
            };
            let admin_single = if group.is_empty() {
                format!("{admin_cms}/{singular}")
            } else {
                format!("{admin_cms}/{group}/{singular}")
            };
            if restful {
                api = api.route(
                    &cms_single,
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |auth, state| single_get_handler(auth, state, key.clone())
                    })
                    .put({
                        let key = registry_key.clone();
                        move |auth, state, data| {
                            single_update_handler(auth, state, data, key.clone())
                        }
                    }),
                );
            } else {
                api = api
                    .route(
                        &cms_single,
                        axum::routing::get({
                            let key = registry_key.clone();
                            move |auth, state| single_get_handler(auth, state, key.clone())
                        }),
                    )
                    .route(
                        &format!("{cms_single}/update"),
                        axum::routing::post({
                            let key = registry_key.clone();
                            move |auth, state, data| {
                                single_update_handler(auth, state, data, key.clone())
                            }
                        }),
                    );
            }
            api = api.route(
                &admin_single,
                axum::routing::get({
                    let key = registry_key.clone();
                    move |state| admin_single_get_handler(state, key.clone())
                }),
            );
        } else if restful {
            api = api
                .route(
                    &cms_prefix,
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |auth, state, params| list_handler(auth, state, key.clone(), params)
                    })
                    .post({
                        let key = registry_key.clone();
                        move |auth, state, data| create_handler(auth, state, key.clone(), data)
                    }),
                )
                .route(
                    &format!("{cms_prefix}/{{id}}"),
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |auth, state, path| get_handler(auth, state, path, key.clone())
                    })
                    .put({
                        let key = registry_key.clone();
                        move |auth, state, path, data| {
                            update_handler(auth, state, path, data, key.clone())
                        }
                    })
                    .delete({
                        let key = registry_key.clone();
                        move |auth, state, path| delete_handler(auth, state, path, key.clone())
                    }),
                )
                .route(
                    &admin_prefix,
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |state, auth, params| {
                            admin_list_handler(state, auth, key.clone(), params)
                        }
                    })
                    .post({
                        let key = registry_key.clone();
                        move |auth, state, data| {
                            admin_create_handler(auth, state, key.clone(), data)
                        }
                    }),
                )
                .route(
                    &format!("{admin_prefix}/{{id}}"),
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |state, auth, path| admin_get_handler(state, auth, path, key.clone())
                    })
                    .put({
                        let key = registry_key.clone();
                        move |auth, state, path, data| {
                            admin_update_handler(auth, state, path, data, key.clone())
                        }
                    })
                    .delete({
                        let key = registry_key.clone();
                        move |auth, state, path| {
                            admin_delete_handler(auth, state, path, key.clone())
                        }
                    }),
                )
                .route(
                    &format!("{admin_prefix}/batch"),
                    axum::routing::post({
                        let key = registry_key.clone();
                        move |auth, state, data| admin_batch_handler(auth, state, key.clone(), data)
                    }),
                )
                .route(
                    &format!("{admin_prefix}/import"),
                    axum::routing::post({
                        let key = registry_key.clone();
                        move |auth, state, query, body| {
                            admin_import_handler(auth, state, key.clone(), query, body)
                        }
                    }),
                )
                .route(
                    &format!("{admin_prefix}/export"),
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |auth, state, query| {
                            admin_export_handler(auth, state, key.clone(), query)
                        }
                    }),
                );
        } else {
            api = api
                .route(
                    &cms_prefix,
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |auth, state, params| list_handler(auth, state, key.clone(), params)
                    }),
                )
                .route(
                    &format!("{cms_prefix}/create"),
                    axum::routing::post({
                        let key = registry_key.clone();
                        move |auth, state, data| create_handler(auth, state, key.clone(), data)
                    }),
                )
                .route(
                    &format!("{cms_prefix}/{{id}}"),
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |auth, state, path| get_handler(auth, state, path, key.clone())
                    }),
                )
                .route(
                    &format!("{cms_prefix}/{{id}}/update"),
                    axum::routing::post({
                        let key = registry_key.clone();
                        move |auth, state, path, data| {
                            update_handler(auth, state, path, data, key.clone())
                        }
                    }),
                )
                .route(
                    &format!("{cms_prefix}/{{id}}/delete"),
                    axum::routing::post({
                        let key = registry_key.clone();
                        move |auth, state, path| delete_handler(auth, state, path, key.clone())
                    }),
                )
                .route(
                    &admin_prefix,
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |state, auth, params| {
                            admin_list_handler(state, auth, key.clone(), params)
                        }
                    }),
                )
                .route(
                    &format!("{admin_prefix}/{{id}}"),
                    axum::routing::get({
                        let key = registry_key.clone();
                        move |state, auth, path| admin_get_handler(state, auth, path, key.clone())
                    }),
                );
        }

        let protocol_names: Vec<String> =
            ct.implements.iter().map(|p| p.name().to_string()).collect();
        let plural_prefix = if group.is_empty() {
            plural.clone()
        } else {
            format!("{group}/{plural}")
        };
        api =
            protocol_registry.register_routes_for(&protocol_names, api, &plural_prefix, admin_cms);

        tracing::debug!(
            "registered CMS routes for content type: {} (group={})",
            ct.singular,
            if group.is_empty() {
                "(default)"
            } else {
                &group
            }
        );
    }

    api
}

/// Parse catch-all path into `(group, segment, optional id)`.
///
/// Resolution strategy:
/// 1. If the first segment is a known group name → `group = segments[0]`, `segment = segments[1]`, `id = segments[2]`
/// 2. Otherwise → `group = ""`, `segment = segments[0]`, `id = segments[1]`
fn resolve_path_segments(
    registry: &crate::content_type::ContentTypeRegistry,
    path: &str,
) -> Option<(String, String, Option<String>)> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    if registry.has_group(segments[0]) && segments.len() >= 2 {
        let group = segments[0].to_string();
        let segment = segments[1].to_string();
        let id = segments.get(2).map(|s| s.to_string());
        Some((group, segment, id))
    } else {
        let segment = segments[0].to_string();
        let id = segments.get(1).map(|s| s.to_string());
        Some((String::new(), segment, id))
    }
}

/// Parse catch-all path into `(group, segment, optional id, optional action)` for non-restful mode.
fn resolve_path_segments_with_action(
    registry: &crate::content_type::ContentTypeRegistry,
    path: &str,
) -> Option<(String, String, Option<String>, Option<String>)> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }

    // Determine if first segment is a group
    let (group, rest) = if registry.has_group(segments[0]) && segments.len() >= 2 {
        (segments[0].to_string(), &segments[1..])
    } else {
        (String::new(), &segments[..])
    };

    let first = rest.first()?.to_string();
    match rest.len() {
        1 => Some((group, first, None, None)),
        2 => match rest[1] {
            "create" | "update" | "delete" => Some((group, first, None, Some(rest[1].to_string()))),
            _ => Some((group, first, Some(rest[1].to_string()), None)),
        },
        3 => Some((
            group,
            first,
            Some(rest[1].to_string()),
            Some(rest[2].to_string()),
        )),
        _ => None,
    }
}

/// Find content type by `(group, singular_or_plural)` segment.
///
/// Returns `(content_type, is_single)`.
fn resolve_content_type(
    registry: &crate::content_type::ContentTypeRegistry,
    group: &str,
    segment: &str,
) -> Option<(Arc<ContentTypeSchema>, bool)> {
    // Try single type first (by singular within group)
    if let Some(ct) = registry.get_in_group(group, segment)
        && ct.is_single()
    {
        return Some((ct, true));
    }
    // Try collection (by plural within group)
    if let Some(ct) = registry.get_by_plural_in_group(group, segment) {
        return Some((ct, false));
    }
    None
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
    let Some((group, segment, id)) = resolve_path_segments(&state.content_type_registry, path)
    else {
        return Err(AppError::not_found("invalid cms path"));
    };

    let Some((ct, is_single)) =
        resolve_content_type(&state.content_type_registry, &group, &segment)
    else {
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
                let result = do_create(state, &ct, data, &save_ctx, &auth).await?;
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
    let Some((group, segment, id, action)) =
        resolve_path_segments_with_action(&state.content_type_registry, path)
    else {
        return Err(AppError::not_found("invalid cms path"));
    };

    let Some((ct, is_single)) =
        resolve_content_type(&state.content_type_registry, &group, &segment)
    else {
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
                let result = do_create(state, &ct, data, &save_ctx, &auth).await?;
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
    auth: AuthUser,
    method: axum::http::Method,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    body: Body,
) -> Result<axum::response::Response, AppError> {
    auth.ensure_admin()?;
    let restful = state.config.api_restful;

    if restful {
        dynamic_admin_cms_dispatch_restful(&state, method, &path, params, body, &auth).await
    } else {
        dynamic_admin_cms_dispatch_simple(&state, method, &path, params, body, &auth).await
    }
}

async fn dynamic_admin_cms_dispatch_restful(
    state: &AppState,
    method: axum::http::Method,
    path: &str,
    params: ListParams,
    body: Body,
    auth: &AuthUser,
) -> Result<axum::response::Response, AppError> {
    let Some((group, segment, id)) = resolve_path_segments(&state.content_type_registry, path)
    else {
        return Err(AppError::not_found("invalid admin cms path"));
    };

    let Some((ct, is_single)) =
        resolve_content_type(&state.content_type_registry, &group, &segment)
    else {
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
            (axum::http::Method::GET, Some(id)) if id == "export" => {
                let format = super::export::ExportFormat::from_str(
                    params
                        .extra
                        .get("format")
                        .map(String::as_str)
                        .unwrap_or("json"),
                )
                .map_err(|_| AppError::BadRequest("unsupported export format".into()))?;
                Ok(do_export(state, &ct, format).await?.into_response())
            }
            (axum::http::Method::GET, Some(id)) => {
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let data = do_admin_get(state, &ct, int_id).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::POST, None) => {
                let bytes = axum::body::to_bytes(body, 16 * 1024 * 1024)
                    .await
                    .map_err(|e| AppError::BadRequest(format!("invalid request body: {e}")))?;
                let data: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
                let save_ctx = SaveContext::from_auth(auth);
                let result = do_create(state, &ct, data, &save_ctx, auth).await?;
                Ok((
                    StatusCode::CREATED,
                    Json(crate::errors::response::ApiResponse::success(result)),
                )
                    .into_response())
            }
            (axum::http::Method::PUT, Some(id)) => {
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let bytes = axum::body::to_bytes(body, 16 * 1024 * 1024)
                    .await
                    .map_err(|e| AppError::BadRequest(format!("invalid request body: {e}")))?;
                let data: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
                let save_ctx = SaveContext::from_auth(auth);
                let result = do_update(state, &ct, int_id, data, &save_ctx, auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            (axum::http::Method::DELETE, Some(id)) => {
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                do_delete(state, &ct, int_id, auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(
                    json!({"deleted": true}),
                ))
                .into_response())
            }
            (axum::http::Method::POST, Some(id)) if id == "batch" => {
                let req = parse_batch_request(body).await?;
                let affected = do_admin_batch(state, &ct, &req.action, &req.ids, auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(
                    BatchResponse::new(&req.action, affected),
                ))
                .into_response())
            }
            (axum::http::Method::POST, Some(id)) if id == "import" => {
                let format = params.extra.get("format").cloned().unwrap_or_default();
                let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024)
                    .await
                    .map_err(|e| AppError::BadRequest(format!("invalid request body: {e}")))?;
                let records = super::import::parse_import_records(&ct, &format, &bytes)?;
                let save_ctx = SaveContext::from_auth(auth);
                let result = do_admin_bulk_create(state, &ct, &records, &save_ctx, auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
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
    body: Body,
    auth: &AuthUser,
) -> Result<axum::response::Response, AppError> {
    let Some((group, segment, id, action)) =
        resolve_path_segments_with_action(&state.content_type_registry, path)
    else {
        return Err(AppError::not_found("invalid admin cms path"));
    };

    let Some((ct, is_single)) =
        resolve_content_type(&state.content_type_registry, &group, &segment)
    else {
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
            (axum::http::Method::GET, Some(id)) if id == "export" => {
                let format = super::export::ExportFormat::from_str(
                    params
                        .extra
                        .get("format")
                        .map(String::as_str)
                        .unwrap_or("json"),
                )
                .map_err(|_| AppError::BadRequest("unsupported export format".into()))?;
                Ok(do_export(state, &ct, format).await?.into_response())
            }
            (axum::http::Method::GET, Some(id)) => {
                let int_id = crate::types::snowflake_id::parse_id(&id)?;
                let data = do_admin_get(state, &ct, int_id).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(data)).into_response())
            }
            (axum::http::Method::POST, Some(id)) if id == "batch" => {
                let req = parse_batch_request(body).await?;
                let affected = do_admin_batch(state, &ct, &req.action, &req.ids, auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(
                    BatchResponse::new(&req.action, affected),
                ))
                .into_response())
            }
            (axum::http::Method::POST, Some(id)) if id == "import" => {
                let format = params.extra.get("format").cloned().unwrap_or_default();
                let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024)
                    .await
                    .map_err(|e| AppError::BadRequest(format!("invalid request body: {e}")))?;
                let records = super::import::parse_import_records(&ct, &format, &bytes)?;
                let save_ctx = SaveContext::from_auth(auth);
                let result = do_admin_bulk_create(state, &ct, &records, &save_ctx, auth).await?;
                Ok(Json(crate::errors::response::ApiResponse::success(result)).into_response())
            }
            _ => Err(AppError::not_found(&format!("{method} {path}"))),
        }
    }
}

// ── Core business logic (shared between fixed and dynamic routes) ──────────────────────

/// Parse and validate a `BatchRequest` from the request body.
async fn parse_batch_request(body: Body) -> Result<BatchRequest, AppError> {
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid request body: {e}")))?;
    if bytes.is_empty() {
        return Err(AppError::BadRequest("request body required".to_string()));
    }
    let req: BatchRequest = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::BadRequest(format!("invalid batch request: {e}")))?;
    validate(&req)?;
    Ok(req)
}

/// Execute a batch operation (`delete`) over multiple CMS records.
///
/// Runs each record through the full delete pipeline (permission check, aspects,
/// soft-delete protocol, cache invalidation) via `do_delete`. Writes are serialized
/// by the global write lock acquired inside each delete transaction.
pub async fn do_admin_batch(
    state: &AppState,
    ct: &ContentTypeSchema,
    action: &str,
    ids: &[String],
    auth: &AuthUser,
) -> Result<usize, AppError> {
    auth.ensure_admin()?;
    let parsed = ids
        .iter()
        .map(|id| crate::types::snowflake_id::parse_id(id))
        .collect::<Result<Vec<SnowflakeId>, _>>()?;

    match action {
        "delete" => {
            let mut affected = 0usize;
            for int_id in parsed {
                do_delete(state, ct, int_id, auth).await?;
                affected += 1;
            }
            Ok(affected)
        }
        _ => Err(AppError::BadRequest(format!(
            "unsupported batch action: {action}"
        ))),
    }
}

pub fn cms_list_cache_key(ct: &ContentTypeSchema, query: &ContentQuery) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    query.page.hash(&mut hasher);
    query.page_size.hash(&mut hasher);
    query.search.hash(&mut hasher);
    query.sort.hash(&mut hasher);
    for f in &query.filters {
        f.field.hash(&mut hasher);
        std::mem::discriminant(&f.op).hash(&mut hasher);
        f.value.to_string().hash(&mut hasher);
    }
    for mf in &query.meta_filters {
        mf.path.hash(&mut hasher);
        std::mem::discriminant(&mf.op).hash(&mut hasher);
        mf.value.hash(&mut hasher);
    }
    if let Some(ref rw) = query.rule_where {
        rw.hash(&mut hasher);
        for p in &query.rule_params {
            p.hash(&mut hasher);
        }
    }
    if let Some(ref inc) = query.include {
        for i in inc {
            i.hash(&mut hasher);
        }
    }
    let h = hasher.finish();
    format!("cms:{}:{h:x}", ct.scope())
}

pub fn cms_detail_cache_key(ct: &ContentTypeSchema, id: SnowflakeId) -> String {
    format!("cms:{}:detail:{id}", ct.scope())
}

fn invalidate_cms_cache(state: &AppState, ct: &ContentTypeSchema) {
    let prefix = format!("cms:{}:", ct.scope());
    state.cms_cache.retain(|k, _| !k.starts_with(&prefix));
}

pub async fn do_list(
    state: &AppState,
    ct: &ContentTypeSchema,
    params: ListParams,
    auth: &AuthUser,
) -> Result<serde_json::Value, AppError> {
    auth.ensure_scope(&ct.scope(), TokenAction::Read)?;
    let repo = ContentRepository::new(state.pool.clone());
    let include = params.include.as_deref().map(parse_include);

    let cached_rule = ct
        .cached_rules
        .as_ref()
        .and_then(|r| build_rule_sql(&r.list, auth, &state.config.rule_engine));
    let (mut rule_where, mut rule_params) = compile_list_filters(
        cached_rule,
        params.filter.as_deref(),
        auth,
        &state.config.rule_engine,
    );

    // Owner access: auto-inject `created_by = <auth_id>`
    if ct.api.list.access == ApiAccess::Owner {
        let uid = auth.user_id().unwrap_or(0);
        let col = COL_CREATED_BY;
        let cond = format!("{col} = {}", crate::db::Driver::ph(1 + rule_params.len()));
        rule_where = Some(match rule_where {
            Some(w) => format!("({w}) AND ({cond})"),
            None => cond,
        });
        rule_params.push(uid.to_string());
    }

    let mut meta_filters: Vec<MetaFilter> = Vec::new();
    let meta_prefix = format!("{COL_META}.");

    for (key, v) in &params.extra {
        if let Some(path) = key.strip_prefix(&meta_prefix) {
            let (path, op) = parse_op_key(path).unwrap_or((path, FilterOp::Eq));
            meta_filters.push(MetaFilter {
                path: path.to_string(),
                op,
                value: v.clone(),
            });
        }
    }

    let filters = parse_field_filters(ct, state, &params.extra, params.status).await;

    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters,
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
    auth.ensure_scope(&ct.scope(), TokenAction::Read)?;
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

    // Owner access: only the creator may view
    if ct.api.get.access == ApiAccess::Owner && !is_owner(&result, auth) {
        return Err(AppError::not_found(&format!("{}/{}", ct.name, id)));
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
    auth: &AuthUser,
) -> Result<serde_json::Value, AppError> {
    auth.ensure_scope(&ct.scope(), TokenAction::Create)?;
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
    auth.ensure_scope(&ct.scope(), TokenAction::Update)?;
    let repo = ContentRepository::new(state.pool.clone());

    let old_record_value = repo.find_by_id(ct, id, None, true).await?;

    if ct.api.update.access == ApiAccess::Owner {
        let rec = old_record_value
            .as_ref()
            .ok_or_else(|| AppError::not_found(&format!("{}/{}", ct.name, id)))?;
        if !is_owner(rec, auth) {
            return Err(AppError::ForbiddenRbac(
                "content-type write access denied".to_string(),
            ));
        }
    }

    if let Some(rules) = ct.cached_rules.as_ref()
        && let Some(rule) = rules.update.filter.as_ref()
        && let Some(ref record) = old_record_value
    {
        let ctx = super::rule_engine::RuleContext::from_auth(auth);
        if !rule.evaluate(record, &ctx, &state.config.rule_engine) {
            return Err(AppError::ForbiddenRbac(
                "content-type write access denied".to_string(),
            ));
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
    auth.ensure_scope(&ct.scope(), TokenAction::Delete)?;
    let repo = ContentRepository::new(state.pool.clone());

    let existing = repo.find_by_id(ct, id, None, true).await?;
    let value = existing.ok_or_else(|| AppError::not_found(&ct.singular))?;

    // Owner access: only the creator may delete
    if ct.api.delete.access == ApiAccess::Owner && !is_owner(&value, auth) {
        return Err(AppError::ForbiddenRbac(
            "content-type access denied".to_string(),
        ));
    }

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
            return Err(AppError::ForbiddenOwnership);
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

    let mut meta_filters: Vec<MetaFilter> = Vec::new();
    let meta_prefix = format!("{COL_META}.");

    for (key, v) in &params.extra {
        if let Some(path) = key.strip_prefix(&meta_prefix) {
            let (path, op) = parse_op_key(path).unwrap_or((path, FilterOp::Eq));
            meta_filters.push(MetaFilter {
                path: path.to_string(),
                op,
                value: v.clone(),
            });
        }
    }

    let filters = parse_field_filters(ct, state, &params.extra, params.status).await;
    let (rule_where, rule_params) = compile_list_filters(
        None,
        params.filter.as_deref(),
        &AuthUser::from_parts(None, crate::models::user::UserRole::Admin, None),
        &state.config.rule_engine,
    );

    let query = ContentQuery {
        page: params.page.unwrap_or(1),
        page_size: params.page_size.unwrap_or(20),
        sort: params.sort,
        filters,
        search: params.search,
        fields: None,
        tenant_id: None,
        include,
        skip_total: params.skip_total.unwrap_or(false),
        rule_where,
        rule_params,
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
    let cache_key = format!("cms:{}:single", ct.registry_key());
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
    let result = do_create(&state, &ct, data, &save_ctx, &auth).await?;
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
    auth: AuthUser,
    type_name: String,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let data = do_admin_list(&state, &ct, params).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn admin_get_handler(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let int_id = crate::types::snowflake_id::parse_id(&id)?;
    let data = do_admin_get(&state, &ct, int_id).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(data)))
}

async fn admin_create_handler(
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
    auth.ensure_admin()?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let save_ctx = SaveContext::from_auth(&auth);
    let result = do_create(&state, &ct, data, &save_ctx, &auth).await?;
    Ok((
        StatusCode::CREATED,
        Json(crate::errors::response::ApiResponse::success(result)),
    ))
}

async fn admin_update_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(data): Json<Value>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let int_id = crate::types::snowflake_id::parse_id(&id)?;
    let save_ctx = SaveContext::from_auth(&auth);
    let result = do_update(&state, &ct, int_id, data, &save_ctx, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(result)))
}

async fn admin_delete_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    type_name: String,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let int_id = crate::types::snowflake_id::parse_id(&id)?;
    do_delete(&state, &ct, int_id, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(
        json!({"deleted": true}),
    )))
}

/// Admin: batch operations (`delete`) over CMS records of a content type.
///
/// POST `/admin/cms/{group}/{plural}/batch` with body `{ action, ids }`.
pub async fn admin_batch_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    type_name: String,
    Json(req): Json<BatchRequest>,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    validate(&req)?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let affected = do_admin_batch(&state, &ct, &req.action, &req.ids, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(
        BatchResponse::new(&req.action, affected),
    )))
}

/// Query parameters for the CMS bulk-import endpoint.
#[derive(Debug, Deserialize)]
pub struct ImportQuery {
    /// File format: `json`, `csv` or `xlsx`.
    pub format: String,
}

/// Admin: bulk-import CMS records of a content type from an uploaded file.
///
/// POST `/admin/cms/{group}/{plural}/import?format=csv` with the raw file bytes
/// as the request body. The file is parsed server-side (JSON/CSV/XLSX) and each
/// record is created through the full create pipeline; failures are collected
/// per-record so a single bad row does not abort the whole import.
pub async fn admin_import_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    type_name: String,
    Query(query): Query<ImportQuery>,
    body: Body,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid request body: {e}")))?;
    let records = super::import::parse_import_records(&ct, &query.format, &bytes)?;
    let save_ctx = SaveContext::from_auth(&auth);
    let result = do_admin_bulk_create(&state, &ct, &records, &save_ctx, &auth).await?;
    Ok(Json(crate::errors::response::ApiResponse::success(result)))
}

/// Query parameters for the CMS full-table export endpoint.
#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    /// Format: `json`, `csv`, `sql` or `xlsx`. Defaults to `json`.
    pub format: Option<String>,
}

/// Response tuple for a streamed export: content-type + content-disposition
/// headers and the chunked body.
type ExportResponse = (
    [(axum::http::header::HeaderName, String); 2],
    axum::body::Body,
);

/// Stream every record of a content type to the response.
///
/// Rows are streamed from the database one at a time and pushed to the
/// response as chunks fill, so tables with hundreds of thousands of records
/// export without buffering everything in memory. The first chunk is awaited
/// before returning so an empty or failed export becomes a normal JSON error
/// instead of an empty download.
pub async fn do_export(
    state: &AppState,
    ct: &ContentTypeSchema,
    format: super::export::ExportFormat,
) -> Result<ExportResponse, AppError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, AppError>>(4);
    let pool = state.pool.clone();
    let ct_thread = ct.clone();

    // The export sink holds non-`Send` state (rust_xlsxwriter uses `Rc`
    // internally), so the whole pipeline runs on a dedicated OS thread with a
    // single-threaded tokio runtime. Chunks are handed back over the channel.
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.blocking_send(Err(AppError::Internal(e.into())));
                return;
            }
        };
        rt.block_on(async move {
            let repo = ContentRepository::new(pool);
            let result: Result<usize, AppError> = (async {
                let mut sink = super::export::ExportSink::new(&ct_thread, format);
                let count = repo
                    .stream_all(&ct_thread, None, |row| {
                        let tx = tx.clone();
                        let send: Result<Vec<u8>, AppError> = sink.write_row(&row);
                        async move {
                            match send {
                                Ok(chunk) if !chunk.is_empty() => {
                                    tx.send(Ok(chunk)).await.map_err(|_| {
                                        AppError::Internal(anyhow::anyhow!("export channel closed"))
                                    })?;
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    let _ = tx.send(Err(e)).await;
                                }
                            }
                            Ok(())
                        }
                    })
                    .await?;
                if count == 0 {
                    return Err(AppError::BadRequest("no records to export".into()));
                }
                let tail = sink.finish()?;
                if !tail.is_empty() {
                    tx.send(Ok(tail)).await.map_err(|_| {
                        AppError::Internal(anyhow::anyhow!("export channel closed"))
                    })?;
                }
                Ok(count)
            })
            .await;
            if let Err(e) = result {
                let _ = tx.send(Err(e)).await;
            }
        });
    });

    // Await the first chunk so an empty/errored export returns a proper JSON
    // error response rather than a streamed empty file.
    let first = match rx.recv().await {
        Some(Ok(chunk)) => chunk,
        Some(Err(e)) => return Err(e),
        None => return Err(AppError::BadRequest("no records to export".into())),
    };

    use futures::StreamExt;
    let rest = tokio_stream::wrappers::ReceiverStream::new(rx);
    let stream = futures::stream::once(async { Ok(axum::body::Bytes::from(first)) })
        .chain(rest.map(|item| item.map(axum::body::Bytes::from)));
    let body = Body::from_stream(stream);

    let filename = super::export::suggested_filename(ct, format);
    let headers = [
        (axum::http::header::CONTENT_TYPE, format.mime().to_string()),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
    ];
    Ok((headers, body))
}

/// Admin: export every record of a content type, streamed.
///
/// GET `/admin/cms/{group}/{plural}/export?format=json`. See [`do_export`].
pub async fn admin_export_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    type_name: String,
    Query(query): Query<ExportQuery>,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let ct = state
        .content_type_registry
        .get(&type_name)
        .ok_or_else(|| AppError::not_found(&type_name))?;
    let format = super::export::ExportFormat::from_str(query.format.as_deref().unwrap_or("json"))
        .map_err(|_| AppError::BadRequest("unsupported export format".into()))?;
    do_export(&state, &ct, format).await
}

/// Execute a bulk create over CMS records, running each through `do_create`.
///
/// Returns created/failed counts plus per-record error details (capped at 20).
pub async fn do_admin_bulk_create(
    state: &AppState,
    ct: &ContentTypeSchema,
    records: &[serde_json::Value],
    save_ctx: &SaveContext,
    auth: &AuthUser,
) -> Result<BulkImportResponse, AppError> {
    auth.ensure_admin()?;
    let mut created = 0usize;
    let mut errors = Vec::new();
    for (i, record) in records.iter().enumerate() {
        match do_create(state, ct, record.clone(), save_ctx, auth).await {
            Ok(_) => created += 1,
            Err(e) => {
                if errors.len() < 20 {
                    errors.push(crate::dto::batch::BulkImportError {
                        index: i,
                        message: strip_error_prefix(&e),
                    });
                }
            }
        }
    }
    Ok(BulkImportResponse {
        action: "create".into(),
        created,
        failed: records.len() - created,
        errors,
    })
}

/// Strip common `AppError` prefixes (e.g. `bad request: `) so import error
/// messages read cleanly in the admin UI.
fn strip_error_prefix(error: &AppError) -> String {
    const PREFIXES: &[&str] = &["bad request: ", "conflict: ", "not found: ", "forbidden: "];
    let msg = error.to_string();
    for prefix in PREFIXES {
        if let Some(rest) = msg.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    msg
}

// ── Schema Management API ──────────────────────────────────────────

/// Parse a catch-all content-type path into a registry key.
///
/// `"post"` → `"post"` (no group)
/// `"forum/poll"` → `"forum/poll"` (with group)
fn parse_ct_path(ct_path: &str) -> String {
    ct_path.trim_start_matches('/').to_string()
}

/// GET /admin/content-types — List schema definitions of all registered content types
pub async fn list_schemas(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let schemas = state.content_type_registry.all();
    Ok(Json(crate::errors::response::ApiResponse::success(schemas)))
}

/// GET /admin/content-types/:ct_path — Get the schema definition of a single content type
///
/// `ct_path` can be `"singular"` (no group) or `"group/singular"` (with group).
pub async fn get_schema(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ct_path): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let key = parse_ct_path(&ct_path);
    let ct = state
        .content_type_registry
        .get(&key)
        .ok_or_else(|| AppError::not_found(&ct_path))?;
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
    auth: AuthUser,
    Json(req): Json<super::schema::CreateContentTypeRequest>,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let schema = super::schema::ContentTypeSchema {
        name: req.name,
        singular: req.singular.clone(),
        plural: req.plural,
        table: req.table.clone(),
        group: super::schema::ContentTypeSchema::validate_group_name(&req.group)
            .map_err(|e| AppError::BadRequest(e.to_string()))?,
        description: req.description,
        icon: req.icon,
        color: req.color,
        kind: req.kind,
        slug_field: req.slug_field,
        search_fields: req.search_fields,
        builtin: req.builtin,
        implements: req.implements,
        fields: req.fields,
        indexes: vec![],
        api: req.api.unwrap_or_default(),
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
            schema.table
        )));
    }

    let registry_key = schema.registry_key();
    if state.content_type_registry.get(&registry_key).is_some() {
        return Err(AppError::Conflict(format!(
            "content type '{}' already exists",
            registry_key
        )));
    }

    state
        .content_type_registry
        .check_relation_targets(&schema)?;

    let dir = std::path::Path::new(&state.config.content_type_dir);
    schema.save_to_dir(dir)?;

    let repo = ContentRepository::new(state.pool.clone());
    repo.migrate(&schema, &state.protocol_registry).await?;

    let reserved = state.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = state.protocol_registry.names();
    state.content_type_registry.register(
        schema.clone(),
        &state.config.rule_engine,
        &reserved,
        &protocol_names,
        &state.protocol_registry,
    )?;

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

/// DELETE /admin/content-types/:ct_path — Delete a content type
///
/// `ct_path` can be `"singular"` (no group) or `"group/singular"` (with group).
/// Deletes the TOML file and unregisters from the in-memory registry. Does not drop the database table.
pub async fn delete_schema(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ct_path): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let key = parse_ct_path(&ct_path);
    let ct = state
        .content_type_registry
        .get(&key)
        .ok_or_else(|| AppError::not_found(&ct_path))?;

    let path = std::path::Path::new(&state.config.content_type_dir).join(ct.toml_filename());
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("cannot delete {:?}: {e}", path)))?;
    }

    state.content_type_registry.unregister(&key);

    tracing::info!("unregistered content type: {} (hot-reload)", key);

    Ok(Json(crate::errors::response::ApiResponse::success(
        serde_json::json!({"deleted": true}),
    )))
}

/// PUT /admin/content-types/:ct_path — Update a content type schema
///
/// `ct_path` can be `"singular"` (no group) or `"group/singular"` (with group).
/// Incremental update: only modifies fields provided in the request. If `fields` is provided,
/// it is compared against the database and automatically `ALTER TABLE ADD COLUMN` to add missing
/// columns (does not delete columns or change column types).
/// The updated schema is synced to the in-memory registry (takes effect immediately, no restart required).
pub async fn update_schema(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ct_path): Path<String>,
    Json(req): Json<super::schema::UpdateContentTypeRequest>,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    let key = parse_ct_path(&ct_path);
    let ct = state
        .content_type_registry
        .get(&key)
        .ok_or_else(|| AppError::not_found(&ct_path))?;

    let mut updated = (*ct).clone();

    if let Some(name) = req.name {
        updated.name = name;
    }
    if let Some(description) = req.description {
        updated.description = description;
    }
    if let Some(icon) = req.icon {
        updated.icon = icon;
    }
    if let Some(color) = req.color {
        updated.color = color;
    }
    if let Some(slug_field) = req.slug_field {
        updated.slug_field = slug_field;
    }
    if let Some(search_fields) = req.search_fields {
        updated.search_fields = search_fields;
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
    if let Some(api) = req.api {
        updated.api = api;
    }

    let dir = std::path::Path::new(&state.config.content_type_dir);
    updated.save_to_dir(dir)?;

    state
        .content_type_registry
        .check_relation_targets(&updated)?;

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
implements = ["ownable"]

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

    fn empty_registry() -> crate::content_type::ContentTypeRegistry {
        crate::content_type::ContentTypeRegistry::new()
    }

    #[test]
    fn resolve_path_segments_empty() {
        let reg = empty_registry();
        assert!(resolve_path_segments(&reg, "").is_none());
    }

    #[test]
    fn resolve_path_segments_flat_plural() {
        let reg = empty_registry();
        let (group, seg, id) = resolve_path_segments(&reg, "products").unwrap();
        assert_eq!(group, "");
        assert_eq!(seg, "products");
        assert!(id.is_none());
    }

    #[test]
    fn resolve_path_segments_flat_with_id() {
        let reg = empty_registry();
        let (group, seg, id) = resolve_path_segments(&reg, "products/abc-123").unwrap();
        assert_eq!(group, "");
        assert_eq!(seg, "products");
        assert_eq!(id, Some("abc-123".to_string()));
    }

    #[test]
    fn resolve_path_segments_batch() {
        let reg = empty_registry();
        let (group, seg, id) = resolve_path_segments(&reg, "products/batch").unwrap();
        assert_eq!(group, "");
        assert_eq!(seg, "products");
        assert_eq!(id, Some("batch".to_string()));
    }

    #[test]
    fn resolve_path_segments_grouped_batch() {
        let reg = crate::content_type::ContentTypeRegistry::new();
        reg.set_protected_tables(vec![]);
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"
implements = ["ownable"]

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        let mut preg = crate::protocols::ProtocolRegistry::new();
        preg.register(crate::protocols::ownable::OwnableProtocol);
        reg.register(ct, &Default::default(), &[], &["ownable"], &preg)
            .unwrap();
        let (group, seg, id) = resolve_path_segments(&reg, "forum/polls/batch").unwrap();
        assert_eq!(group, "forum");
        assert_eq!(seg, "polls");
        assert_eq!(id, Some("batch".to_string()));
    }

    #[test]
    fn resolve_path_segments_grouped() {
        let reg = crate::content_type::ContentTypeRegistry::new();
        reg.set_protected_tables(vec![]);
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"
implements = ["ownable"]

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        let mut preg = crate::protocols::ProtocolRegistry::new();
        preg.register(crate::protocols::ownable::OwnableProtocol);
        reg.register(ct, &Default::default(), &[], &["ownable"], &preg)
            .unwrap();

        let (group, seg, id) = resolve_path_segments(&reg, "forum/polls").unwrap();
        assert_eq!(group, "forum");
        assert_eq!(seg, "polls");
        assert!(id.is_none());

        let (group, seg, id) = resolve_path_segments(&reg, "forum/polls/123").unwrap();
        assert_eq!(group, "forum");
        assert_eq!(seg, "polls");
        assert_eq!(id, Some("123".to_string()));
    }

    #[test]
    fn resolve_path_segments_with_action_flat_create() {
        let reg = empty_registry();
        let (group, seg, id, action) =
            resolve_path_segments_with_action(&reg, "products/create").unwrap();
        assert_eq!(group, "");
        assert_eq!(seg, "products");
        assert!(id.is_none());
        assert_eq!(action, Some("create".to_string()));
    }

    #[test]
    fn resolve_path_segments_with_action_flat_id_update() {
        let reg = empty_registry();
        let (group, seg, id, action) =
            resolve_path_segments_with_action(&reg, "products/abc-123/update").unwrap();
        assert_eq!(group, "");
        assert_eq!(seg, "products");
        assert_eq!(id, Some("abc-123".to_string()));
        assert_eq!(action, Some("update".to_string()));
    }

    #[test]
    fn resolve_path_segments_with_action_empty() {
        let reg = empty_registry();
        assert!(resolve_path_segments_with_action(&reg, "").is_none());
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
implements = ["ownable"]

[fields.key]
type = "text"
"#,
        )
        .unwrap();
        let mut reg = crate::protocols::ProtocolRegistry::new();
        reg.register(crate::protocols::ownable::OwnableProtocol);
        let _ = registry.register(ct, &Default::default(), &[], &["ownable"], &reg);
        let (found, is_single) = resolve_content_type(&registry, "", "setting").unwrap();
        assert!(is_single);
        assert_eq!(found.singular, "setting");
    }

    #[test]
    fn resolve_content_type_by_plural() {
        let registry = crate::content_type::ContentTypeRegistry::new();
        let ct = parse_ct();
        let mut reg = crate::protocols::ProtocolRegistry::new();
        reg.register(crate::protocols::ownable::OwnableProtocol);
        let _ = registry.register(ct, &Default::default(), &[], &["ownable"], &reg);
        let (found, is_single) = resolve_content_type(&registry, "", "products").unwrap();
        assert!(!is_single);
        assert_eq!(found.singular, "product");
    }

    #[test]
    fn resolve_content_type_not_found() {
        let registry = crate::content_type::ContentTypeRegistry::new();
        assert!(resolve_content_type(&registry, "", "nothing").is_none());
    }

    #[test]
    fn resolve_content_type_grouped() {
        let registry = crate::content_type::ContentTypeRegistry::new();
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"
implements = ["ownable"]

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        let mut reg = crate::protocols::ProtocolRegistry::new();
        reg.register(crate::protocols::ownable::OwnableProtocol);
        registry
            .register(ct, &Default::default(), &[], &["ownable"], &reg)
            .unwrap();

        let (found, is_single) = resolve_content_type(&registry, "forum", "polls").unwrap();
        assert!(!is_single);
        assert_eq!(found.singular, "poll");
        assert_eq!(found.group, "forum");
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
    fn cms_cache_key_group_qualified() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"
implements = ["ownable"]

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        // Grouped: scope = "forum/polls"
        assert_eq!(ct.scope(), "forum/polls");
        let key = cms_detail_cache_key(&ct, SnowflakeId(42));
        assert_eq!(key, "cms:forum/polls:detail:42");
    }

    #[test]
    fn cms_list_cache_key_contains_plural() {
        let ct = parse_ct();
        let query = crate::content_type::repository::ContentQuery {
            page: 1,
            page_size: 20,
            sort: None,
            filters: Default::default(),
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

    #[test]
    fn parse_op_key_parses_operator_suffix() {
        assert_eq!(parse_op_key("price[$gt]"), Some(("price", FilterOp::Gt)));
        assert_eq!(parse_op_key("status[$in]"), Some(("status", FilterOp::In)));
        assert_eq!(
            parse_op_key("title[$contains]"),
            Some(("title", FilterOp::Contains))
        );
        assert_eq!(
            parse_op_key("price[$between]"),
            Some(("price", FilterOp::Between))
        );
        assert_eq!(
            parse_op_key("created_at[$is_null]"),
            Some(("created_at", FilterOp::IsNull))
        );
        assert_eq!(parse_op_key("name"), None);
        assert_eq!(parse_op_key("bad[$unknown]"), None);
        assert_eq!(parse_op_key("bad[$gt"), None);
    }

    #[test]
    fn parse_op_key_does_not_treat_meta_as_field() {
        // Meta keys are handled separately; bracket suffix still parsed.
        let ct = parse_ct();
        assert!(!ct.has_column("__meta.views"));
    }

    #[test]
    fn compile_list_filters_merges_cached_and_user_filter() {
        let config = crate::config::app::RuleEngineConfig::default();
        let anon = AuthUser::from_parts(None, crate::models::user::UserRole::Reader, None);

        // cached rule only
        let cached = Some(("status = 'published'".to_string(), vec!["published".into()]));
        let (w, p) = compile_list_filters(cached, None, &anon, &config);
        assert_eq!(w.as_deref(), Some("status = 'published'"));
        assert_eq!(p.len(), 1);

        // user filter only
        let (w, p) = compile_list_filters(None, Some("price >= 100"), &anon, &config);
        let w = w.unwrap();
        assert!(w.contains("\"price\" >= "));
        assert_eq!(p.len(), 1);

        // both — AND-ed with param offset preserved
        let cached = Some(("status = 'published'".to_string(), vec!["published".into()]));
        let (w, p) = compile_list_filters(cached, Some("price >= 100"), &anon, &config);
        let w = w.unwrap();
        assert!(w.starts_with("(status = 'published') AND (\"price\" >= "));
        assert_eq!(p.len(), 2);

        // malformed filter is ignored
        let cached = Some(("status = 'published'".to_string(), vec!["published".into()]));
        let (w, p) = compile_list_filters(cached, Some("price >=> 100"), &anon, &config);
        assert_eq!(w.as_deref(), Some("status = 'published'"));
        assert_eq!(p.len(), 1);
    }
}

//! `ct` node executor — first-class CRUD on content types (design: CT CRUD
//! node, [自造] vs n8n/dify raw-SQL nodes: dynamic schema + tenant isolation
//! + soft-delete/ownable protocols come free with the CT repository).
//!
//! Shared runtime (registry + repository) is installed once at startup
//! (mirrors `integration::set_shared`); the per-run tenant rides on
//! `FlowsExec.tenant_id`. Writes go through the repository which acquires the
//! platform write lock itself.

use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value, json};

use crate::content_type::repository::{
    ContentQuery, ContentRepository, FieldFilter, FilterOp, SaveContext,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;

use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;

/// One filter row of a `ct` node: `value` is a C3.1 template.
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CtFilterRow {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: String,
}

/// One payload field of a `ct` node (`insert`/`update`): `value` is a template.
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CtSetRow {
    pub field: String,
    #[serde(default)]
    pub value: String,
}

/// `ct` node config — first-class CRUD on a content type (tenant isolation
/// and soft-delete/ownable protocols inherited from the CT repository).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CtConfig {
    /// Content-type plural name.
    pub content_type: String,
    pub op: String,
    #[serde(default)]
    pub filters: Vec<CtFilterRow>,
    #[serde(default)]
    pub sort: Option<String>,
    /// Record id (template) — required for `update` / `delete`.
    #[serde(default)]
    pub id: Option<String>,
    /// Payload rows for `insert` / `update`.
    #[serde(default)]
    pub values: Vec<CtSetRow>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub page: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub page_size: Option<i64>,
}

/// Operations allowed on the `ct` node.
pub const CT_OPS: &[&str] = &[
    "find_one",
    "find_page",
    "count",
    "insert",
    "update",
    "delete",
];

/// Filter operators exposed to flow authors (subset of repository FilterOp).
pub const CT_FILTER_OPS: &[&str] = &["eq", "ne", "gt", "gte", "lt", "lte", "contains", "like"];

/// Max rows a `find_page` may return in one run (safety cap).
pub const CT_MAX_PAGE_SIZE: i64 = 100;

/// Config validation (moved from the central registry; media-nodes.md §5).
pub(super) fn validate(config: &serde_json::Value) -> AppResult<()> {
    let c: CtConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'ct' config invalid: {e}")))?;
    if c.content_type.trim().is_empty() {
        return Err(AppError::BadRequest("ct: content_type 不能为空".into()));
    }
    if !CT_OPS.contains(&c.op.as_str()) {
        return Err(AppError::BadRequest(format!(
            "ct: op '{}' 非法（允许: {}）",
            c.op,
            CT_OPS.join("/")
        )));
    }
    if matches!(c.op.as_str(), "update" | "delete")
        && c.id.as_deref().is_none_or(|v| v.trim().is_empty())
    {
        return Err(AppError::BadRequest(format!("ct: op '{}' 需要 id", c.op)));
    }
    for f in &c.filters {
        if !crate::db::driver::is_safe_identifier(&f.field) {
            return Err(AppError::BadRequest(format!(
                "ct: filters.field '{}' 非法标识符",
                f.field
            )));
        }
        if !CT_FILTER_OPS.contains(&f.op.as_str()) {
            return Err(AppError::BadRequest(format!(
                "ct: filters.op '{}' 非法（允许: {}）",
                f.op,
                CT_FILTER_OPS.join("/")
            )));
        }
    }
    for v in &c.values {
        if !crate::db::driver::is_safe_identifier(&v.field) {
            return Err(AppError::BadRequest(format!(
                "ct: values.field '{}' 非法标识符",
                v.field
            )));
        }
    }
    if let Some(sort) = &c.sort {
        let field = sort.trim_start_matches('-');
        if !crate::db::driver::is_safe_identifier(field) || field.is_empty() {
            return Err(AppError::BadRequest(format!(
                "ct: sort '{}' 非法（field 或 -field）",
                sort
            )));
        }
    }
    if c.page.is_some_and(|p| p < 1) || c.page_size.is_some_and(|p| p < 1) {
        return Err(AppError::BadRequest("ct: page/page_size 须 ≥1".into()));
    }
    if c.page_size.is_some_and(|p| p > CT_MAX_PAGE_SIZE) {
        return Err(AppError::BadRequest(format!(
            "ct: page_size 上限 {CT_MAX_PAGE_SIZE}"
        )));
    }
    Ok(())
}

pub struct CtRuntime {
    pub registry: Arc<crate::content_type::ContentTypeRegistry>,
    pub protocols: Arc<crate::protocols::ProtocolRegistry>,
    pub repo: ContentRepository,
}

static SHARED_CT: OnceLock<Arc<CtRuntime>> = OnceLock::new();

/// Install the shared CT runtime (called once from `build_app_state`).
pub fn set_shared_ct(runtime: Arc<CtRuntime>) {
    let _ = SHARED_CT.set(runtime);
}

/// Access the shared CT runtime, if initialized.
#[must_use]
pub fn shared_ct() -> Option<Arc<CtRuntime>> {
    SHARED_CT.get().cloned()
}

fn filter_op(op: &str) -> Option<FilterOp> {
    Some(match op {
        "eq" => FilterOp::Eq,
        "ne" => FilterOp::Ne,
        "gt" => FilterOp::Gt,
        "gte" => FilterOp::Gte,
        "lt" => FilterOp::Lt,
        "lte" => FilterOp::Lte,
        "contains" => FilterOp::Contains,
        "like" => FilterOp::Like,
        _ => return None,
    })
}

/// Render a template to a typed Value (whole-string refs keep type — a number
/// filter value stays a number).
fn render_value(text: &str, pool: &Pool) -> AppResult<Value> {
    crate::flows::expr::resolve_text(text, pool)
}

fn render_id(template: &str, pool: &Pool) -> AppResult<SnowflakeId> {
    let v = render_value(template, pool)?;
    let id = match &v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s
            .parse::<i64>()
            .ok()
            .or_else(|| crate::types::snowflake_id::parse_id(s).ok().map(|i| *i)),
        _ => None,
    }
    .ok_or_else(|| AppError::BadRequest(format!("ct: id 无效: {v}")))?;
    Ok(SnowflakeId::from(id))
}

fn build_query(cfg: &CtConfig, pool: &Pool, page_size_cap: i64) -> AppResult<ContentQuery> {
    let mut filters = Vec::with_capacity(cfg.filters.len());
    for f in &cfg.filters {
        if f.value.is_empty() && matches!(f.op.as_str(), "eq" | "ne") {
            continue;
        }
        let Some(op) = filter_op(&f.op) else {
            return Err(AppError::BadRequest(format!("ct: 过滤操作符非法 {}", f.op)));
        };
        filters.push(FieldFilter {
            field: f.field.clone(),
            op,
            value: render_value(&f.value, pool)?,
        });
    }
    Ok(ContentQuery {
        page: cfg.page.unwrap_or(1).max(1),
        page_size: cfg.page_size.unwrap_or(20).clamp(1, page_size_cap.max(1)),
        sort: cfg.sort.clone(),
        filters,
        tenant_id: None,
        max_page_size: CT_MAX_PAGE_SIZE,
        ..ContentQuery::default()
    })
}

/// Execute the `ct` node against the variable pool.
///
/// # Errors
/// `BadRequest` on unknown content type / template failures / invalid id;
/// repository errors surface as-is.
pub async fn run_ct(
    node: &GraphNode,
    pool: &Pool,
    tenant_id: Option<&str>,
) -> AppResult<ExecOutcome> {
    let cfg: CtConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("ct config: {e}")))?;

    let runtime =
        shared_ct().ok_or_else(|| AppError::BadRequest("ct: 内容类型运行时不可用".into()))?;
    let ct = runtime
        .registry
        .get_by_plural(cfg.content_type.trim())
        .ok_or_else(|| {
            AppError::BadRequest(format!("ct: 内容类型 '{}' 不存在", cfg.content_type))
        })?;

    let mut out = Map::new();
    match cfg.op.as_str() {
        "find_one" => {
            let mut q = build_query(&cfg, pool, 1)?;
            q.page = 1;
            q.page_size = 1;
            q.tenant_id = tenant_id.map(str::to_string);
            let (rows, _) = runtime.repo.find(&ct, q).await?;
            out.insert(
                "record".into(),
                rows.into_iter().next().unwrap_or(Value::Null),
            );
        }
        "find_page" => {
            let mut q = build_query(&cfg, pool, CT_MAX_PAGE_SIZE)?;
            q.tenant_id = tenant_id.map(str::to_string);
            let (items, total) = runtime.repo.find(&ct, q).await?;
            out.insert("items".into(), Value::Array(items));
            out.insert("total".into(), json!(total));
        }
        "count" => {
            let mut q = build_query(&cfg, pool, 1)?;
            q.page = 1;
            q.page_size = 1;
            q.tenant_id = tenant_id.map(str::to_string);
            let (_, total) = runtime.repo.find(&ct, q).await?;
            out.insert("total".into(), json!(total));
        }
        "insert" => {
            let mut data = Map::new();
            for v in &cfg.values {
                if !v.field.is_empty() {
                    data.insert(v.field.clone(), render_value(&v.value, pool)?);
                }
            }
            let record = runtime
                .repo
                .create(
                    &ct,
                    Value::Object(data),
                    tenant_id,
                    &SaveContext {
                        tenant_id: tenant_id.map(str::to_string),
                        ..SaveContext::default()
                    },
                )
                .await?;
            out.insert("record".into(), record);
        }
        "update" => {
            let id_template = cfg.id.clone().unwrap_or_default();
            let id = render_id(&id_template, pool)?;
            let mut data = Map::new();
            for v in &cfg.values {
                if !v.field.is_empty() {
                    data.insert(v.field.clone(), render_value(&v.value, pool)?);
                }
            }
            let record = runtime
                .repo
                .update(
                    &ct,
                    id,
                    Value::Object(data),
                    tenant_id,
                    &SaveContext {
                        tenant_id: tenant_id.map(str::to_string),
                        ..SaveContext::default()
                    },
                )
                .await?;
            out.insert("record".into(), record);
        }
        "delete" => {
            let id_template = cfg.id.clone().unwrap_or_default();
            let id = render_id(&id_template, pool)?;
            if ct.is_soft_delete() {
                runtime
                    .repo
                    .soft_delete(&ct, id, &crate::utils::tz::now_str(), None, tenant_id)
                    .await?;
            } else {
                runtime
                    .repo
                    .delete(&ct, id, tenant_id, &runtime.protocols, &runtime.registry)
                    .await?;
            }
            out.insert("affected".into(), json!(1));
        }
        other => {
            return Err(AppError::BadRequest(format!("ct: op '{other}' 未实现")));
        }
    }
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: None,
        latency_ms: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::graph::NodeData;

    fn ct_node(config: Value) -> GraphNode {
        GraphNode {
            id: "n1".into(),
            data: NodeData {
                kind: "ct".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn unknown_content_type_is_bad_request() {
        let err = run_ct(
            &ct_node(json!({"content_type": "nope", "op": "find_one"})),
            &Pool::new(),
            None,
        )
        .await
        .unwrap_err();
        // No shared runtime in unit tests → runtime-unavailable error.
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }

    #[test]
    fn filter_op_mapping() {
        assert!(filter_op("eq").is_some());
        assert!(filter_op("like").is_some());
        assert!(filter_op("drop table").is_none());
    }
}

//! Install precheck (Resolve, app-bundle.md §4.3) — collect every conflict
//! up front, report the full structured list, only then materialize.
//!
//! Also implements the keep-data re-attach rule (§4.3 修订 #2): a physical
//! table that exists while its CT is unregistered is a residue of this same
//! app's keep-data uninstall — precheck verifies schema compatibility and
//! lets the install take the table over instead of rejecting it.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;

use crate::apps::manifest::permission_covers;
use crate::apps::package::AppPackage;
use crate::config::app::AppConfig;
use crate::content_type::ContentTypeRegistry;
use crate::db::DbDriver;
use crate::errors::app_error::AppResult;
use crate::integration::channel::ItgChannel;

/// One conflict item for the install wizard.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Conflict {
    pub code: &'static str,
    pub detail: String,
    /// `block` = cannot install; `warn` = needs acknowledgement.
    pub severity: &'static str,
}

/// The full precheck report (serializable for `install-preview`).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PrecheckReport {
    pub conflicts: Vec<Conflict>,
    /// Residual tables detected for re-attach (informational).
    pub reattach_tables: Vec<String>,
}

impl PrecheckReport {
    #[must_use]
    pub fn blocking(&self) -> Vec<&Conflict> {
        self.conflicts
            .iter()
            .filter(|c| c.severity == "block")
            .collect()
    }

    #[must_use]
    pub fn ok(&self) -> bool {
        self.blocking().is_empty()
    }

    fn block(&mut self, code: &'static str, detail: impl Into<String>) {
        self.conflicts.push(Conflict {
            code,
            detail: detail.into(),
            severity: "block",
        });
    }

    fn warn(&mut self, code: &'static str, detail: impl Into<String>) {
        self.conflicts.push(Conflict {
            code,
            detail: detail.into(),
            severity: "warn",
        });
    }
}

/// Context needed by precheck.
pub struct PrecheckCtx<'a> {
    pub pool: &'a crate::db::Pool,
    pub config: &'a AppConfig,
    pub ct_registry: &'a Arc<ContentTypeRegistry>,
    /// Existing registered plugin ids (from the plugin manager).
    pub registered_plugins: Vec<String>,
    /// Registered plugin route paths: (method, path).
    pub plugin_routes: Vec<(String, String)>,
}

/// Run the full precheck matrix (§4.3). Pure read — never mutates.
pub async fn run(pkg: &AppPackage, ctx: &PrecheckCtx<'_>) -> AppResult<PrecheckReport> {
    let mut report = PrecheckReport::default();
    let manifest = &pkg.manifest;
    let prefix = manifest.table_prefix();

    // ── platform version ────────────────────────────────────────
    if let Some(req) = &manifest.app.requires_raisfast
        && !crate::apps::manifest::check_requirement(req, env!("CARGO_PKG_VERSION"))
    {
        report.block(
            "requires-raisfast",
            format!(
                "app requires raisfast {req}, this host is {}",
                env!("CARGO_PKG_VERSION")
            ),
        );
    }

    // ── already installed / busy ─────────────────────────────────
    if let Some(row) = crate::apps::model::model::find_by_app_id(ctx.pool, &manifest.app.id).await?
    {
        report.block(
            "app-exists",
            format!(
                "app '{}' already installed (version {}, status {})",
                row.app_id, row.version, row.status
            ),
        );
    }

    // ── hard dependency (§7 — install check only, no resolver) ──
    if let Some(req) = &manifest.dependencies.requires {
        let installed = crate::apps::model::model::find_by_app_id(ctx.pool, &req.app).await?;
        match installed {
            None => report.block(
                "dependency-missing",
                format!("requires app '{}' which is not installed", req.app),
            ),
            Some(row) => {
                if let Some(need) = &req.version
                    && crate::apps::manifest::cmp_semver(need, &row.version)
                        .map(|ord| ord.is_gt())
                        .unwrap_or(true)
                {
                    report.block(
                        "dependency-version",
                        format!(
                            "requires app '{}' {}, installed {}",
                            req.app, need, row.version
                        ),
                    );
                }
            }
        }
    }

    // ── content types: prefix, group, table collisions, re-attach ─
    let mut seen_tables = BTreeSet::new();
    for ct in &pkg.content_types {
        let schema = &ct.schema;
        if !schema.table.starts_with(&prefix) {
            report.block(
                "table-prefix",
                format!(
                    "content type '{}' table '{}' must start with the app prefix '{prefix}'",
                    schema.name, schema.table
                ),
            );
        }
        if schema.group != manifest.app.id {
            report.block(
                "ct-group",
                format!(
                    "content type '{}' group must be '{}' (found '{}')",
                    schema.name, manifest.app.id, schema.group
                ),
            );
        }
        if !seen_tables.insert(schema.table.clone()) {
            report.block(
                "duplicate-table",
                format!("package declares table '{}' twice", schema.table),
            );
        }

        // Registered-CT conflict (someone else owns the name).
        if ctx.ct_registry.get_by_table(&schema.table).is_some() {
            report.block(
                "table-registered",
                format!(
                    "table '{}' is registered by another content type",
                    schema.table
                ),
            );
            continue;
        }
        if crate::db::schema::get_protected_tables().contains(&schema.table) {
            report.block(
                "table-protected",
                format!("table '{}' is a protected host table", schema.table),
            );
            continue;
        }

        // Physical-table analysis (re-attach rule, §4.3).
        if crate::db::Driver::table_exists(ctx.pool, &schema.table).await {
            match residual_compatible(ctx.pool, &schema.table, schema).await? {
                ResidualVerdict::Reattach => {
                    report.reattach_tables.push(schema.table.clone());
                    report.warn(
                        "reattach",
                        format!(
                            "table '{}' exists unregistered — treated as keep-data residue of \
                             this app; install will re-attach it",
                            schema.table
                        ),
                    );
                }
                ResidualVerdict::Foreign => {
                    report.block(
                        "table-exists",
                        format!(
                            "table '{}' exists but does not match this app's schema (foreign \
                             table or incompatible residue — use an upgrade migration instead)",
                            schema.table
                        ),
                    );
                }
            }
        }

        // group/singular/plural uniqueness against registered CTs.
        let key = schema.registry_key();
        if ctx.ct_registry.get(&key).is_some() {
            report.block(
                "ct-key",
                format!("content type key '{key}' already registered"),
            );
        }
        if let Some(conflict) = ctx
            .ct_registry
            .get_by_plural_in_group(&schema.group, &schema.plural)
            .map(|ct| ct.name.clone())
        {
            report.block(
                "ct-plural",
                format!(
                    "plural '{}' in group '{}' already used by '{conflict}'",
                    schema.plural, schema.group
                ),
            );
        }
    }

    // ── plugins: id + route conflicts ────────────────────────────
    for plugin in &pkg.plugins {
        let effective_id = manifest.plugin_id(&plugin.manifest.plugin.id);
        if ctx.registered_plugins.iter().any(|p| p == &effective_id) {
            report.block(
                "plugin-id",
                format!("plugin '{effective_id}' is already registered"),
            );
        }
        for route in &plugin.manifest.routes {
            let path = format!("/{}", route.path.trim_start_matches('/'));
            if ctx
                .plugin_routes
                .iter()
                .any(|(m, p)| m == &route.method && p == &path)
            {
                report.block(
                    "route-conflict",
                    format!(
                        "route {} {} conflicts with an existing plugin route",
                        route.method, path
                    ),
                );
            }
        }

        // Permission convergence (§2.3): plugin-declared permissions ⊆ app
        // permissions.
        for host in &plugin.manifest.permissions.http {
            let domain = host.split('/').next().unwrap_or(host);
            if !domain.is_empty()
                && !permission_covers(&manifest.app.permissions, &format!("http:{domain}"))
            {
                report.block(
                    "permission-convergence",
                    format!(
                        "plugin '{}' declares http host '{host}' not covered by app permissions",
                        plugin.manifest.plugin.id
                    ),
                );
            }
        }
        for client in &plugin.manifest.permissions.egress {
            if client != "*"
                && !permission_covers(&manifest.app.permissions, &format!("http:{client}"))
            {
                report.block(
                    "permission-convergence",
                    format!(
                        "plugin '{}' declares egress client '{client}' not covered by app \
                         permissions",
                        plugin.manifest.plugin.id
                    ),
                );
            }
        }
        for entry in &plugin.manifest.cron {
            let covered = permission_covers(&manifest.app.permissions, "cron:*")
                || permission_covers(
                    &manifest.app.permissions,
                    &format!("cron:{}", entry.job_type),
                );
            if !covered {
                report.block(
                    "permission-convergence",
                    format!(
                        "plugin '{}' declares cron job '{}' not covered by app permissions",
                        plugin.manifest.plugin.id, entry.job_type
                    ),
                );
            }
        }
    }

    // ── channel key collisions (UNIQUE(tenant_id, channel_key, version)) ─
    let tenant = crate::constants::DEFAULT_TENANT;
    for seed in &pkg.channel_seeds {
        let key = seed
            .get("channel_key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let existing =
            crate::integration::channel::model::find_by_key(ctx.pool, tenant, key).await?;
        if ItgChannel::resolve_active(&existing).is_some() {
            report.block(
                "channel-key",
                format!("channel_key '{key}' already exists in tenant '{tenant}'"),
            );
        }
    }

    // ── api-client key collisions (UNIQUE(tenant_id, client_key)) ─
    for seed in &pkg.api_client_seeds {
        let key = seed
            .get("client_key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if crate::apps::precheck::api_client_key_exists(ctx.pool, tenant, key).await? {
            report.block(
                "api-client-key",
                format!("api client key '{key}' already exists in tenant '{tenant}'"),
            );
        }
    }

    // ── CT seeds target known app tables ─────────────────────────
    let app_tables: BTreeSet<&str> = pkg
        .content_types
        .iter()
        .map(|c| c.schema.table.as_str())
        .collect();
    for seed in &pkg.ct_seeds {
        if !app_tables.contains(seed.table.as_str()) {
            report.block(
                "seed-table",
                format!(
                    "seed file targets table '{}' which is not declared by this package",
                    seed.table
                ),
            );
        }
    }

    // ── options namespace (app.{app_id}.* enforced, §9) ──────────
    for opt in &pkg.option_seeds {
        if let Some(key) = opt.get("key").and_then(Value::as_str)
            && !key.starts_with(&format!("app.{}.", manifest.app.id))
        {
            report.block(
                "option-prefix",
                format!(
                    "option key '{key}' must use the 'app.{}.*' prefix",
                    manifest.app.id
                ),
            );
        }
    }

    Ok(report)
}

async fn api_client_key_exists(pool: &crate::db::Pool, tenant: &str, key: &str) -> AppResult<bool> {
    let sql = format!(
        "SELECT id FROM itg_api_clients WHERE tenant_id = {} AND client_key = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2)
    );
    let row: Option<(i64,)> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(tenant)
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

enum ResidualVerdict {
    /// Columns are a subset of (or compatible with) the new schema — the
    /// install may mount the table (repo.migrate adds missing columns).
    Reattach,
    /// Table has columns the new schema cannot explain — foreign or drifted.
    Foreign,
}

/// A residual table is re-attachable when every existing column is either a
/// CT field, a protocol column or a system column, and every shared column's
/// type family matches the declared field type (loose comparison across
/// backends' type spellings).
async fn residual_compatible(
    pool: &crate::db::Pool,
    table: &str,
    schema: &crate::content_type::schema::ContentTypeSchema,
) -> AppResult<ResidualVerdict> {
    use crate::content_type::schema::FieldType;

    let columns = crate::db::Driver::fetch_columns_with_types(pool, table)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "cannot inspect residual table '{table}': {e}"
            ))
        })?;

    let mut expected: std::collections::HashMap<String, FieldType> = schema
        .fields
        .iter()
        .map(|f| (f.name.clone(), f.field_type.clone()))
        .collect();
    // Relation FK columns decode as bigint-ish.
    for field in &schema.fields {
        if let Some(rel) = &field.relation {
            let fk = rel
                .foreign_key
                .clone()
                .unwrap_or_else(|| format!("{}_id", field.name));
            expected.insert(fk, FieldType::BigInt);
        }
    }
    let system: &[&str] = &[
        "id",
        "tenant_id",
        "created_at",
        "updated_at",
        "created_by",
        "updated_by",
        "version",
        "deleted_at",
        "seed_key",
    ];

    for (col, sql_type) in &columns {
        if let Some(field_type) = expected.remove(col) {
            if !type_family_matches(&field_type, sql_type) {
                return Ok(ResidualVerdict::Foreign);
            }
        } else if !system.contains(&col.as_str()) {
            // A column neither the schema nor the platform knows — drift.
            return Ok(ResidualVerdict::Foreign);
        }
    }
    Ok(ResidualVerdict::Reattach)
}

/// Loose type-family comparison across backends (`TEXT` vs `VARCHAR(255)` vs
/// `CLOB`, `BIGINT` vs `INTEGER`, ...).
fn type_family_matches(
    field_type: &crate::content_type::schema::FieldType,
    sql_type: &str,
) -> bool {
    use crate::content_type::schema::FieldType;
    let t = sql_type.to_lowercase();
    let text_like =
        || t.contains("text") || t.contains("varchar") || t.contains("char") || t.contains("clob");
    let int_like = || t.contains("int") || t.contains("serial");
    let float_like = || {
        t.contains("real")
            || t.contains("double")
            || t.contains("float")
            || t.contains("numeric")
            || t.contains("decimal")
    };
    match field_type {
        FieldType::Integer | FieldType::BigInt => int_like(),
        FieldType::Decimal | FieldType::Float => float_like() || int_like(),
        FieldType::Boolean => t.contains("bool") || t.contains("int") || t.contains("bit"),
        FieldType::Date => t.contains("date") && !t.contains("time"),
        FieldType::DateTime => t.contains("date") || t.contains("time") || text_like(),
        FieldType::Time => t.contains("time") || text_like(),
        FieldType::Json => t.contains("json") || text_like(),
        _ => text_like() || t.contains("json"),
    }
}

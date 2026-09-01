//! AppRegistry — the lifecycle state-machine holder and orchestration entry
//! point (app-bundle.md §3). The DB is the source of truth; the registry
//! adds the runtime handles (plugin manager, integration plane) and the
//! startup reconciliation (self-heal + CT replay).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::apps::installer::{self, InstallCtx, InstallTarget, Progress};
use crate::apps::manifest::AppBundleManifest;
use crate::apps::model::{self, AppRow, model as app_model};
use crate::apps::package::AppPackage;
use crate::apps::precheck::{self, PrecheckCtx, PrecheckReport};
use crate::apps::uninstaller::{self, UndoCtx};
use crate::config::app::AppConfig;
use crate::content_type::ContentTypeRegistry;
use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::integration::IntegrationPlane;
use crate::plugins::PluginManager;
use crate::protocols::ProtocolRegistry;

/// Lifecycle orchestration facade. Cheap to share (`Arc`), created once at
/// startup ([`AppRegistry::init`]) and stored in the process-wide handle
/// ([`crate::apps::set_shared`]).
pub struct AppRegistry {
    pool: crate::db::Pool,
    config: Arc<AppConfig>,
    ct_registry: Arc<ContentTypeRegistry>,
    protocol_registry: Arc<ProtocolRegistry>,
    plugins: RwLock<Option<Arc<PluginManager>>>,
    plane: RwLock<Option<Arc<IntegrationPlane>>>,
}

/// Install-time wizard options.
#[derive(Debug, Default, serde::Deserialize)]
pub struct InstallOptions {
    /// Override the manifest's `uninstall-keep-data` default.
    #[serde(default)]
    pub keep_data: Option<bool>,
}

impl AppRegistry {
    /// Early startup phase: self-heal残留 + replay app CTs into the registry.
    /// Runs BEFORE the CT migrate loop and before the plugin manager exists.
    pub async fn init(
        pool: crate::db::Pool,
        config: Arc<AppConfig>,
        ct_registry: Arc<ContentTypeRegistry>,
        protocol_registry: Arc<ProtocolRegistry>,
    ) -> AppResult<Arc<Self>> {
        let tmp_root = config.storage_root_dir.clone() + "/apps/tmp";
        std::fs::create_dir_all(&tmp_root)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("create {tmp_root}: {e}")))?;

        let registry = Arc::new(Self {
            pool,
            config,
            ct_registry,
            protocol_registry,
            plugins: RwLock::new(None),
            plane: RwLock::new(None),
        });
        registry.self_heal().await?;
        registry.replay_app_cts().await?;
        Ok(registry)
    }

    /// Late startup phase: attach runtime handles and reconcile plugin
    /// state with app status (load_all picks up app plugin dirs from disk;
    /// non-enabled apps must not keep routes alive).
    pub async fn attach(
        &self,
        plugins: Arc<PluginManager>,
        plane: Option<Arc<IntegrationPlane>>,
    ) -> AppResult<()> {
        *self.plugins.write().await = Some(plugins);
        *self.plane.write().await = plane;
        self.reconcile_plugins().await
    }

    fn undo_ctx(&self) -> UndoCtx {
        let plugins = self.plugins.try_read().ok().and_then(|g| g.clone());
        let plane = self.plane.try_read().ok().and_then(|g| g.clone());
        UndoCtx {
            pool: self.pool.clone(),
            config: self.config.clone(),
            ct_registry: self.ct_registry.clone(),
            plugins,
            plane,
        }
    }

    async fn install_ctx(&self) -> InstallCtx {
        let plugins = self.plugins.read().await.clone();
        let plane = self.plane.read().await.clone();
        InstallCtx {
            pool: self.pool.clone(),
            config: self.config.clone(),
            ct_registry: self.ct_registry.clone(),
            protocol_registry: self.protocol_registry.clone(),
            plugins,
            plane,
        }
    }

    async fn precheck_ctx(&self) -> AppResult<PrecheckCtx<'_>> {
        let plugins = self.plugins.read().await.clone();
        let (registered_plugins, plugin_routes) = match &plugins {
            Some(p) => {
                let ids = p
                    .list_plugins()
                    .await
                    .into_iter()
                    .map(|(id, _, _)| id)
                    .collect();
                let routes = p
                    .all_plugin_routes()
                    .await
                    .into_iter()
                    .map(|(m, path, _, _)| (m, path))
                    .collect();
                (ids, routes)
            }
            None => (Vec::new(), Vec::new()),
        };
        Ok(PrecheckCtx {
            pool: &self.pool,
            config: &self.config,
            ct_registry: &self.ct_registry,
            registered_plugins,
            plugin_routes,
        })
    }

    /// Directory an upload unpacks into (cleaned by the caller).
    #[must_use]
    pub fn unpack_dir(&self, token: &str) -> PathBuf {
        PathBuf::from(self.config.storage_root_dir.clone())
            .join("apps")
            .join("tmp")
            .join(token)
    }

    // ── Lifecycle operations ──────────────────────────────────────

    /// Precheck only — the install wizard's conflict report (no mutation).
    pub async fn install_preview(&self, pkg: &AppPackage) -> AppResult<PrecheckReport> {
        let ctx = self.precheck_ctx().await?;
        precheck::run(pkg, &ctx).await
    }

    /// Full install: precheck → install row (installing) → 8-step
    /// materialization → installed. Any failure rolls back every done step
    /// in reverse and lands in `rolled_back` with `last_error`.
    pub async fn install(&self, pkg: &AppPackage, opts: &InstallOptions) -> AppResult<Value> {
        // Resolve (§4.3): full conflict report before any DDL.
        let ctx = self.precheck_ctx().await?;
        let report = precheck::run(pkg, &ctx).await?;
        if !report.ok() {
            return Err(AppError::Conflict(conflict_summary(&report)));
        }

        let keep_data = opts
            .keep_data
            .unwrap_or(pkg.manifest.install.uninstall_keep_data);

        // Claim the app row (UNIQUE(app_id) guards concurrent installs).
        let row = AppRow {
            id: crate::utils::id::new_snowflake_id(),
            app_id: pkg.manifest.app.id.clone(),
            version: pkg.manifest.app.version.clone(),
            status: model::STATUS_INSTALLING.to_string(),
            source: "upload".into(),
            source_ref: None,
            signature_ok: true,
            install_log: None,
            last_error: None,
            tenant_scope: pkg.manifest.install.tenant_scope.clone(),
            options: Some(serde_json::json!({ "keep_data": keep_data })),
            installed_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        };
        app_model::insert(&self.pool, &row).await?;

        let target = InstallTarget {
            app_id: pkg.manifest.app.id.clone(),
            version: pkg.manifest.app.version.clone(),
            keep_data,
        };
        let ictx = self.install_ctx().await;
        let mut progress = Progress::new(self.pool.clone(), &target.app_id);

        match installer::materialize(&ictx, &target, pkg, &mut progress).await {
            Ok(pending) => {
                let mut options = serde_json::json!({ "keep_data": keep_data });
                if !pending.items.is_empty() {
                    options["pending_credentials"] = serde_json::Value::Array(
                        pending
                            .items
                            .iter()
                            .map(|i| Value::String(i.clone()))
                            .collect(),
                    );
                }
                let result = raisfast_derive::crud_update!(
                    &self.pool,
                    "apps",
                    bind: [
                        "status" => model::STATUS_INSTALLED,
                        "options" => options,
                        "install_log" => serde_json::json!({ "steps": progress.steps() }),
                        "updated_at" => crate::utils::tz::now_utc()
                    ],
                    where: ("app_id", target.app_id.as_str())
                )?;
                if result.rows_affected() == 0 {
                    return Err(AppError::Conflict("app row vanished during install".into()));
                }
                tracing::info!(
                    "app '{}' {} installed ({} steps)",
                    pkg.manifest.app.id,
                    pkg.manifest.app.version,
                    progress.steps().len()
                );
                Ok(serde_json::json!({
                    "app_id": pkg.manifest.app.id,
                    "version": pkg.manifest.app.version,
                    "status": model::STATUS_INSTALLED,
                    "steps": progress.steps().len(),
                    "pending_credentials": pending.items,
                    "reattach_tables": report.reattach_tables,
                }))
            }
            Err(e) => {
                // Compensate: reverse undo of every done step. A failed
                // install returns the system to its pre-install state —
                // half-created tables are dropped (keep_data is an uninstall
                // concern, not a rollback one).
                let uctx = self.undo_ctx();
                uninstaller::run_undo_reverse(&uctx, progress.steps(), false).await?;
                let msg = format!("{e:#}");
                let _ = app_model::set_error(
                    &self.pool,
                    &target.app_id,
                    model::STATUS_ROLLED_BACK,
                    &msg,
                )
                .await;
                tracing::error!("app '{}' install failed, rolled back: {msg}", target.app_id);
                Err(AppError::Internal(anyhow::anyhow!(
                    "install failed and was rolled back: {msg}"
                )))
            }
        }
    }

    /// Enable: channels up, plugins loaded + crons synced.
    pub async fn enable(&self, app_id: &str) -> AppResult<()> {
        let row = self.require_row(app_id).await?;
        app_model::ensure_not_busy(&row)?;
        if !app_model::cas_status(
            &self.pool,
            app_id,
            &[model::STATUS_INSTALLED, model::STATUS_DISABLED],
            model::STATUS_ENABLED,
        )
        .await?
        {
            return Err(AppError::Conflict(format!(
                "app '{app_id}' cannot transition from '{}' to enabled (concurrent operation?)",
                row.status
            )));
        }

        let steps = uninstaller::parse_log(row.install_log.as_ref());
        let uctx = self.undo_ctx();

        // Plugins up (materialized dirs are the truth for what to load).
        if let Some(plugins) = &uctx.plugins {
            for dir in uninstaller::plugin_dirs(&steps) {
                let manifest = std::path::Path::new(&dir).join("manifest.toml");
                if manifest.exists()
                    && let Err(e) = plugins.load_plugin_from_dir(&manifest).await
                {
                    tracing::warn!("app '{app_id}' plugin load {dir:?} failed: {e}");
                }
            }
        }
        // Channels up + caches refreshed.
        let ids = uninstaller::channel_ids(&steps);
        eprintln!("DBG enable steps={} ids={:?}", steps.len(), ids);
        if !ids.is_empty() {
            let plane = self.plane.read().await.clone();
            channel_set_enabled(&self.pool, plane.as_ref(), &ids, true).await?;
        }
        tracing::info!("app '{app_id}' enabled");
        Ok(())
    }

    /// Disable: drain orchestration (routes down → 60s window → dead-letter
    /// remainder), then plugins unloaded + crons removed. Pure logical
    /// switch — physical structure untouched.
    pub async fn disable(&self, app_id: &str) -> AppResult<()> {
        let row = self.require_row(app_id).await?;
        app_model::ensure_not_busy(&row)?;
        if !app_model::cas_status(
            &self.pool,
            app_id,
            &[model::STATUS_ENABLED],
            model::STATUS_DISABLED,
        )
        .await?
        {
            return Err(AppError::Conflict(format!(
                "app '{app_id}' is not enabled (status '{}')",
                row.status
            )));
        }

        let steps = uninstaller::parse_log(row.install_log.as_ref());
        let uctx = self.undo_ctx();

        // Routes down + drain first (§3.1: never recycle while in flight).
        let ids = uninstaller::channel_ids(&steps);
        if !ids.is_empty() {
            let plane = self.plane.read().await.clone();
            let drain =
                uninstaller::drain_channels(&self.pool, plane.as_ref(), &ids, self.drain_window())
                    .await?;
            if !drain.drained_clean {
                tracing::warn!(
                    "app '{app_id}' drain timed out: {} receipt(s) dead-lettered",
                    drain.timeout_dead
                );
            }
        }
        // Plugins down.
        if let Some(plugins) = &uctx.plugins {
            for plugin_id in uninstaller::plugin_ids(&steps) {
                plugins.unload_plugin(&plugin_id).await;
                let _ = crate::worker::remove_plugin_crons(&self.pool, &plugin_id).await;
            }
        }
        tracing::info!("app '{app_id}' disabled");
        Ok(())
    }

    /// Uninstall: drain → reverse recovery of every step → row deleted.
    /// `keep_data` defaults to the install-time choice (§3.1: Gone |
    /// Data-kept — data-kept leaves tables for re-attach).
    pub async fn uninstall(&self, app_id: &str, keep_data: Option<bool>) -> AppResult<()> {
        let row = self.require_row(app_id).await?;
        app_model::ensure_not_busy(&row)?;
        if !app_model::cas_status(
            &self.pool,
            app_id,
            &[
                model::STATUS_INSTALLED,
                model::STATUS_ENABLED,
                model::STATUS_DISABLED,
            ],
            model::STATUS_UNINSTALLING,
        )
        .await?
        {
            return Err(AppError::Conflict(format!(
                "app '{app_id}' cannot transition from '{}' to uninstalling (concurrent \
                 operation?)",
                row.status
            )));
        }

        match self.uninstall_inner(&row, keep_data, true).await {
            Ok(()) => {
                tracing::info!("app '{app_id}' uninstalled");
                Ok(())
            }
            Err(e) => {
                // Stay in `uninstalling` — the restart self-heal resumes.
                let msg = format!("{e:#}");
                let _ = app_model::set_error(&self.pool, app_id, model::STATUS_UNINSTALLING, &msg)
                    .await;
                Err(AppError::Internal(anyhow::anyhow!(
                    "uninstall incomplete (will self-heal at restart): {msg}"
                )))
            }
        }
    }

    /// The destructive part of uninstall; shared with restart self-heal.
    async fn uninstall_inner(
        &self,
        row: &AppRow,
        keep_data: Option<bool>,
        drain: bool,
    ) -> AppResult<()> {
        let steps = uninstaller::parse_log(row.install_log.as_ref());
        let keep = keep_data.unwrap_or_else(|| option_keep_data(row));

        if drain {
            let ids = uninstaller::channel_ids(&steps);
            if !ids.is_empty() {
                let plane = self.plane.read().await.clone();
                uninstaller::drain_channels(&self.pool, plane.as_ref(), &ids, self.drain_window())
                    .await?;
            }
        }

        let uctx = self.undo_ctx();
        uninstaller::run_undo_reverse(&uctx, &steps, keep).await?;
        app_model::delete_licenses_by_app(&self.pool, &row.app_id).await?;
        app_model::delete_by_app_id(&self.pool, &row.app_id).await?;
        Ok(())
    }

    async fn require_row(&self, app_id: &str) -> AppResult<AppRow> {
        app_model::find_by_app_id(&self.pool, app_id)
            .await?
            .ok_or_else(|| AppError::not_found(&format!("app/{app_id}")))
    }

    // ── Startup reconciliation ────────────────────────────────────

    /// Kill -9 self-heal (§3.2): `installing`残留 → rollback → rolled_back;
    /// `uninstalling`残留 → complete the uninstall (in-flight envelopes
    /// cannot exist pre-serve, so drain is skipped).
    async fn self_heal(&self) -> AppResult<()> {
        for row in app_model::find_by_status(&self.pool, model::STATUS_INSTALLING).await? {
            let steps = uninstaller::parse_log(row.install_log.as_ref());
            let uctx = self.undo_ctx();
            uninstaller::run_undo_reverse(&uctx, &steps, false).await?;
            let _ = app_model::set_error(
                &self.pool,
                &row.app_id,
                model::STATUS_ROLLED_BACK,
                "install interrupted (kill -9); compensation replayed at restart",
            )
            .await;
            tracing::warn!(
                "app '{}' recovered from interrupted install → rolled-back",
                row.app_id
            );
        }
        for row in app_model::find_by_status(&self.pool, model::STATUS_UNINSTALLING).await? {
            if let Err(e) = self.uninstall_inner(&row, None, false).await {
                tracing::error!("self-heal uninstall '{}' failed: {e}", row.app_id);
            } else {
                tracing::warn!("app '{}' uninstall completed by self-heal", row.app_id);
            }
        }
        Ok(())
    }

    /// Replay app CTs from `app_ct_refs` (§6.3 three-source rebuild: host
    /// builtins → directory scan (already done) → this). Orphan refs whose
    /// app row is gone are pruned.
    async fn replay_app_cts(&self) -> AppResult<()> {
        let refs = app_model::all_ct_refs(&self.pool).await?;
        let apps: std::collections::HashSet<String> = app_model::find_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| r.app_id)
            .collect();

        let reserved = self.config.builtins.reserved_route_segments();
        let protocol_names = self.protocol_registry.names();
        for r in refs {
            if !apps.contains(&r.app_id) {
                let _ = app_model::delete_ct_refs_by_app(&self.pool, &r.app_id).await;
                continue;
            }
            match crate::content_type::schema::ContentTypeSchema::parse_from_str(&r.schema_toml) {
                Ok(schema) => {
                    if let Err(e) = self.ct_registry.register(
                        schema,
                        &self.config.rule_engine,
                        &reserved,
                        &protocol_names,
                        &self.protocol_registry,
                    ) {
                        tracing::warn!("app ct '{}/{}' replay failed: {e}", r.app_id, r.ct_table);
                    }
                }
                Err(e) => tracing::warn!(
                    "app ct '{}/{}' TOML no longer parses: {e}",
                    r.app_id,
                    r.ct_table
                ),
            }
        }
        Ok(())
    }

    /// Plugin state reconciliation: `load_all` loads everything on disk;
    /// apps that are not `enabled` must have their plugins unloaded again.
    async fn reconcile_plugins(&self) -> AppResult<()> {
        let rows = app_model::find_all(&self.pool).await?;
        let disabled: Vec<AppRow> = rows
            .into_iter()
            .filter(|r| r.status != model::STATUS_ENABLED)
            .collect();
        if disabled.is_empty() {
            return Ok(());
        }
        let Some(plugins) = self.plugins.read().await.clone() else {
            return Ok(());
        };
        for row in disabled {
            let steps = uninstaller::parse_log(row.install_log.as_ref());
            for plugin_id in uninstaller::plugin_ids(&steps) {
                plugins.unload_plugin(&plugin_id).await;
            }
        }
        Ok(())
    }

    fn drain_window(&self) -> Duration {
        Duration::from_secs(self.config.apps.drain_window_secs.max(1))
    }

    // ── Read accessors for handlers ───────────────────────────────

    #[must_use]
    pub fn pool(&self) -> &crate::db::Pool {
        &self.pool
    }

    /// Installed apps with lifecycle summaries.
    pub async fn list(&self) -> AppResult<Vec<AppRow>> {
        app_model::find_all(&self.pool).await
    }

    pub async fn detail(&self, app_id: &str) -> AppResult<AppRow> {
        self.require_row(app_id).await
    }
}

async fn channel_set_enabled(
    pool: &crate::db::Pool,
    plane: Option<&Arc<IntegrationPlane>>,
    ids: &[i64],
    enabled: bool,
) -> AppResult<()> {
    let placeholders = (3..=ids.len() + 2)
        .map(crate::db::Driver::ph)
        .collect::<Vec<_>>()
        .join(", ");
    // Placeholder order = SQL appearance order (MySQL binds positionally):
    // enabled, updated_at, then ids. PG numbering matches (1, 2, 3..).
    let sql = format!(
        "UPDATE itg_channels SET enabled = {p1}, updated_at = {p2} WHERE id IN ({placeholders})",
        placeholders = placeholders,
        p1 = crate::db::Driver::ph(1),
        p2 = crate::db::Driver::ph(2),
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql))
        .bind(enabled)
        .bind(crate::utils::tz::now_utc());
    for id in ids {
        q = q.bind(id);
    }
    q.execute(pool).await?;
    if let Some(plane) = plane {
        plane.channels().refresh().await?;
        plane.wake_supervisor();
    }
    Ok(())
}

fn option_keep_data(row: &AppRow) -> bool {
    row.options
        .as_ref()
        .and_then(|o| o.get("keep_data"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn conflict_summary(report: &PrecheckReport) -> String {
    let blocking: Vec<String> = report
        .blocking()
        .iter()
        .map(|c| format!("[{}] {}", c.code, c.detail))
        .collect();
    format!(
        "install precheck failed with {} blocking conflict(s): {}",
        blocking.len(),
        blocking.join("; ")
    )
}

/// Convenience for handlers/tests: the shared registry handle.
#[must_use]
pub fn shared() -> Option<Arc<AppRegistry>> {
    crate::apps::shared()
}

/// Manifest accessor used by the admin layer.
pub type Manifest = AppBundleManifest;

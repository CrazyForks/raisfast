//! `apps` / `app_ct_refs` row models and CRUD (app-bundle.md §3.2, §6.3).
//!
//! Lifecycle operations are mutually exclusive via CAS on `status`
//! (`WHERE status IN (...)` + `rows_affected()` — the cross-DB claim
//! pattern; MySQL has no RETURNING).

use serde::Serialize;
use serde_json::Value;

use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

// ── Status machine values ───────────────────────────────────────────

pub const STATUS_INSTALLING: &str = "installing";
pub const STATUS_INSTALLED: &str = "installed";
pub const STATUS_ENABLED: &str = "enabled";
pub const STATUS_DISABLED: &str = "disabled";
pub const STATUS_UNINSTALLING: &str = "uninstalling";
pub const STATUS_ROLLED_BACK: &str = "rolled_back";

/// Statuses during which every concurrent lifecycle op is rejected (409).
pub const BUSY_STATUSES: &[&str] = &[STATUS_INSTALLING, STATUS_UNINSTALLING];

/// One compensation-log entry (app-bundle.md §4.2). The undo descriptor is
/// generated at the same place as the step itself and persisted after every
/// step so a kill -9 restart can replay compensation.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct LogStep {
    pub seq: usize,
    pub step: String,
    pub detail: String,
    pub undo: UndoAction,
    pub done: bool,
}

/// Re-executable undo descriptors. Rollback / uninstall / restart self-heal
/// all run the same executor (reverse order, best-effort).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UndoAction {
    /// Delete an rbac role (and its permission rows) created by the app.
    /// IDs are strings on purpose: JSON-number round-trips lose Snowflake
    /// precision on some backends (MySQL JSON → double).
    DropRole { role_id: String },
    /// Unregister a CT from the registry (never touches data).
    UnregisterCt { key: String },
    /// Drop the physical CT table + junction tables (skipped on keep-data).
    DropTable {
        table: String,
        junctions: Vec<String>,
    },
    /// Delete the materialized plugin directory.
    DeletePluginDir { dir: String },
    /// Unload a plugin from the manager (no-op when not loaded).
    UnloadPlugin { plugin_id: String },
    /// Remove the plugin's synced cron schedules.
    RemoveCrons { plugin_id: String },
    /// Delete a seeded integration channel row.
    DeleteChannel { id: String },
    /// Delete a seeded api-client row.
    DeleteApiClient { id: String },
    /// Delete seed rows identified by their `seed_key` values.
    DeleteSeedRows {
        table: String,
        seed_keys: Vec<String>,
    },
    /// Delete option rows inserted by the app (`app.{app_id}.*`).
    DeleteOptions { keys: Vec<String> },
    /// Delete roles created by `seeds/roles.json` (namespaced).
    DropSeedRole { role_id: String },
    /// Delete the `app_ct_refs` row for one CT.
    DeleteCtRef { table: String },
    /// Informational step with nothing to undo (e.g. admin-page declarations
    /// while the SPA renderer is pending).
    Noop,
}

/// Row of the `apps` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AppRow {
    pub id: SnowflakeId,
    pub app_id: String,
    pub version: String,
    pub status: String,
    pub source: String,
    pub source_ref: Option<String>,
    pub signature_ok: bool,
    pub install_log: Option<Value>,
    pub last_error: Option<String>,
    pub tenant_scope: String,
    pub options: Option<Value>,
    pub installed_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Row of the `app_ct_refs` table — the authoritative CT definition an app
/// shipped, replayed at startup to rebuild the registry (§6.3).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AppCtRefRow {
    pub id: SnowflakeId,
    pub app_id: String,
    pub ct_table: String,
    pub schema_toml: String,
    pub version: String,
    pub created_at: Timestamp,
}

#[allow(clippy::module_inception)]
pub mod model {
    use super::{AppCtRefRow, AppRow, BUSY_STATUSES};
    use crate::db::DbDriver;
    use crate::errors::app_error::{AppError, AppResult};
    use crate::types::snowflake_id::SnowflakeId;
    use crate::utils::tz::now_utc;

    const APP_COLS: &str = "id, app_id, version, status, source, source_ref, signature_ok, \
         install_log, last_error, tenant_scope, options, installed_at, updated_at";

    pub async fn insert(pool: &crate::db::Pool, row: &AppRow) -> AppResult<()> {
        raisfast_derive::crud_insert!(
            pool,
            "apps",
            [
                "id" => row.id,
                "app_id" => &row.app_id,
                "version" => &row.version,
                "status" => &row.status,
                "source" => &row.source,
                "source_ref" => row.source_ref.as_deref(),
                "signature_ok" => row.signature_ok,
                "install_log" => row.install_log.as_ref(),
                "last_error" => row.last_error.as_deref(),
                "tenant_scope" => &row.tenant_scope,
                "options" => row.options.as_ref(),
                "installed_at" => now_utc(),
                "updated_at" => now_utc()
            ]
        )?;
        Ok(())
    }

    pub async fn find_by_app_id(pool: &crate::db::Pool, app_id: &str) -> AppResult<Option<AppRow>> {
        Ok(raisfast_derive::crud_find!(pool, "apps", AppRow, where: ("app_id", app_id))?)
    }

    pub async fn find_by_status(pool: &crate::db::Pool, status: &str) -> AppResult<Vec<AppRow>> {
        Ok(raisfast_derive::crud_find_all!(pool, "apps", AppRow, where: ("status", status))?)
    }

    pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<AppRow>> {
        let sql = format!("SELECT {APP_COLS} FROM apps ORDER BY installed_at");
        let rows: Vec<AppRow> =
            sqlx::query_as::<crate::db::pool::Db, AppRow>(crate::db::safe_sql(&sql))
                .fetch_all(pool)
                .await?;
        Ok(rows)
    }

    /// CAS status flip: succeeds only when the current status is one of
    /// `expected`. `Ok(false)` = concurrent modification (caller maps to 409).
    pub async fn cas_status(
        pool: &crate::db::Pool,
        app_id: &str,
        expected: &[&str],
        to: &str,
    ) -> AppResult<bool> {
        let mut sql = format!(
            "UPDATE apps SET status = {}, updated_at = {} WHERE app_id = {} AND status IN (",
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3),
        );
        for (i, _) in expected.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&crate::db::Driver::ph(4 + i));
        }
        sql.push(')');

        let mut q = sqlx::query(crate::db::safe_sql(&sql))
            .bind(to)
            .bind(now_utc())
            .bind(app_id);
        for e in expected {
            q = q.bind(e);
        }
        let result = q.execute(pool).await?;
        Ok(result.rows_affected() > 0)
    }

    /// Reject lifecycle ops against busy apps with a 409 carrying the busy
    /// status (app-bundle.md §3.2 操作互斥).
    pub fn ensure_not_busy(row: &AppRow) -> AppResult<()> {
        if BUSY_STATUSES.contains(&row.status.as_str()) {
            return Err(AppError::Conflict(format!(
                "app '{}' is busy (status: {}); concurrent lifecycle operations are rejected",
                row.app_id, row.status
            )));
        }
        Ok(())
    }

    /// Persist the compensation log (called after every step append).
    pub async fn update_install_log(
        pool: &crate::db::Pool,
        app_id: &str,
        log: &serde_json::Value,
    ) -> AppResult<()> {
        let result = raisfast_derive::crud_update!(
            pool,
            "apps",
            bind: ["install_log" => log, "updated_at" => now_utc()],
            where: ("app_id", app_id)
        )?;
        AppError::expect_affected(&result, "apps")?;
        Ok(())
    }

    pub async fn set_error(
        pool: &crate::db::Pool,
        app_id: &str,
        status: &str,
        last_error: &str,
    ) -> AppResult<()> {
        let result = raisfast_derive::crud_update!(
            pool,
            "apps",
            bind: ["status" => status, "last_error" => last_error, "updated_at" => now_utc()],
            where: ("app_id", app_id)
        )?;
        AppError::expect_affected(&result, "apps")?;
        Ok(())
    }

    pub async fn delete_by_app_id(pool: &crate::db::Pool, app_id: &str) -> AppResult<()> {
        let result = raisfast_derive::crud_delete!(pool, "apps", where: ("app_id", app_id))?;
        AppError::expect_affected(&result, "apps")?;
        Ok(())
    }

    // ── app_ct_refs ────────────────────────────────────────────────

    pub async fn insert_ct_ref(pool: &crate::db::Pool, row: &AppCtRefRow) -> AppResult<()> {
        raisfast_derive::crud_insert!(
            pool,
            "app_ct_refs",
            [
                "id" => row.id,
                "app_id" => &row.app_id,
                "ct_table" => &row.ct_table,
                "schema_toml" => &row.schema_toml,
                "version" => &row.version,
                "created_at" => now_utc()
            ]
        )?;
        Ok(())
    }

    pub async fn ct_refs_by_app(
        pool: &crate::db::Pool,
        app_id: &str,
    ) -> AppResult<Vec<AppCtRefRow>> {
        Ok(
            raisfast_derive::crud_find_all!(pool, "app_ct_refs", AppCtRefRow,
                where: ("app_id", app_id))?,
        )
    }

    /// All refs — startup registry rebuild input.
    pub async fn all_ct_refs(pool: &crate::db::Pool) -> AppResult<Vec<AppCtRefRow>> {
        let sql = "SELECT id, app_id, ct_table, schema_toml, version, created_at FROM \
             app_ct_refs ORDER BY app_id, ct_table";
        let rows: Vec<AppCtRefRow> =
            sqlx::query_as::<crate::db::pool::Db, AppCtRefRow>(crate::db::safe_sql(sql))
                .fetch_all(pool)
                .await?;
        Ok(rows)
    }

    pub async fn delete_ct_refs_by_app(pool: &crate::db::Pool, app_id: &str) -> AppResult<()> {
        raisfast_derive::crud_delete!(pool, "app_ct_refs", where: ("app_id", app_id))?;
        Ok(())
    }

    pub async fn delete_ct_ref(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
        raisfast_derive::crud_delete!(pool, "app_ct_refs", where: ("id", id))?;
        Ok(())
    }

    // ── app_licenses (B4 will build on these) ─────────────────────

    pub async fn delete_licenses_by_app(pool: &crate::db::Pool, app_id: &str) -> AppResult<()> {
        raisfast_derive::crud_delete!(pool, "app_licenses", where: ("app_id", app_id))?;
        Ok(())
    }
}

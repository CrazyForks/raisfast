//! Uninstall: shared undo executor + drain orchestration (app-bundle.md
//! §3.1 下线排空编排, §4.2 rollback).
//!
//! Rollback (install failure), uninstall and restart self-heal all replay
//! the same undo descriptors in reverse order, best-effort — an undo that
//! itself fails is logged and the recovery continues; the final state is
//! written back truthfully (`apps.status` + `last_error`).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::apps::model::{LogStep, UndoAction};
use crate::config::app::AppConfig;
use crate::content_type::ContentTypeRegistry;
use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::integration::IntegrationPlane;
use crate::plugins::PluginManager;

/// Drain window: how long in-flight envelopes may take to reach a terminal
/// receipt state before the remainder is dead-lettered (`drained:timeout`).
pub const DEFAULT_DRAIN_WINDOW: Duration = Duration::from_secs(60);

/// Handles the undo executor needs.
pub struct UndoCtx {
    pub pool: crate::db::Pool,
    pub config: Arc<AppConfig>,
    pub ct_registry: Arc<ContentTypeRegistry>,
    pub plugins: Option<Arc<PluginManager>>,
    pub plane: Option<Arc<IntegrationPlane>>,
}

/// Run one undo action (best-effort; caller logs failures).
pub async fn run_undo(ctx: &UndoCtx, action: &UndoAction, keep_data: bool) -> AppResult<()> {
    match action {
        UndoAction::DropRole { role_id } => drop_role(&ctx.pool, parse_id(role_id)?).await,
        UndoAction::DropSeedRole { role_id } => drop_role(&ctx.pool, parse_id(role_id)?).await,
        UndoAction::UnregisterCt { key } => {
            ctx.ct_registry.unregister(key);
            Ok(())
        }
        UndoAction::DropTable { table, junctions } => {
            if keep_data {
                return Ok(()); // keep-data: physical tables survive (re-attach)
            }
            for t in std::iter::once(table).chain(junctions.iter()) {
                if !crate::db::driver::is_safe_identifier(t) {
                    continue;
                }
                let sql = format!("DROP TABLE IF EXISTS {t}");
                sqlx::query(crate::db::safe_sql(&sql))
                    .execute(&ctx.pool)
                    .await?;
            }
            Ok(())
        }
        UndoAction::DeletePluginDir { dir } => {
            let path = std::path::Path::new(dir);
            // Belt-and-braces: only ever delete inside the plugin root.
            let root = ctx
                .config
                .plugin_dir
                .clone()
                .unwrap_or_else(|| "./extensions/plugins".into());
            if path.starts_with(&root) && path.exists() {
                std::fs::remove_dir_all(path)
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("remove {dir}: {e}")))?;
            }
            Ok(())
        }
        UndoAction::UnloadPlugin { plugin_id } => {
            if let Some(plugins) = &ctx.plugins {
                plugins.unload_plugin(plugin_id).await;
            }
            Ok(())
        }
        UndoAction::RemoveCrons { plugin_id } => {
            crate::worker::remove_plugin_crons(&ctx.pool, plugin_id).await
        }
        UndoAction::DeleteChannel { id } => {
            let row = crate::integration::channel::model::find_by_id(
                &ctx.pool,
                crate::types::snowflake_id::SnowflakeId(parse_id(id)?),
            )
            .await;
            if let Ok(ch) = row {
                crate::integration::channel::model::delete_by_id(&ctx.pool, ch.id).await?;
                if let Some(plane) = &ctx.plane {
                    plane.channels().refresh().await?;
                    plane.wake_supervisor();
                }
            }
            Ok(())
        }
        UndoAction::DeleteApiClient { id } => {
            crate::integration::api_client::model::delete_by_id(
                &ctx.pool,
                crate::types::snowflake_id::SnowflakeId(parse_id(id)?),
            )
            .await
        }
        UndoAction::DeleteSeedRows { table, seed_keys } => {
            if seed_keys.is_empty() {
                return Ok(());
            }
            let placeholders = (1..=seed_keys.len())
                .map(crate::db::Driver::ph)
                .collect::<Vec<_>>();
            let sql = format!(
                "DELETE FROM {table} WHERE seed_key IN ({})",
                placeholders.join(", ")
            );
            let mut q = sqlx::query(crate::db::safe_sql(&sql));
            for key in seed_keys {
                q = q.bind(key);
            }
            q.execute(&ctx.pool).await?;
            Ok(())
        }
        UndoAction::DeleteOptions { keys } => {
            if keys.is_empty() {
                return Ok(());
            }
            let placeholders = (1..=keys.len())
                .map(crate::db::Driver::ph)
                .collect::<Vec<_>>();
            let sql = format!(
                "DELETE FROM options WHERE option_key IN ({})",
                placeholders.join(", ")
            );
            let mut q = sqlx::query(crate::db::safe_sql(&sql));
            for key in keys {
                q = q.bind(key);
            }
            q.execute(&ctx.pool).await?;
            Ok(())
        }
        UndoAction::DeleteCtRef { table } => {
            let sql = format!(
                "DELETE FROM app_ct_refs WHERE ct_table = {}",
                crate::db::Driver::ph(1)
            );
            sqlx::query(crate::db::safe_sql(&sql))
                .bind(table)
                .execute(&ctx.pool)
                .await?;
            Ok(())
        }
        UndoAction::Noop => Ok(()),
    }
}

fn parse_id(raw: &str) -> AppResult<i64> {
    raw.parse::<i64>().map_err(|_| {
        AppError::Internal(anyhow::anyhow!(
            "compensation log carries a malformed id '{raw}'"
        ))
    })
}

async fn drop_role(pool: &crate::db::Pool, role_id: i64) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "permissions",
        where: ("role_id", role_id)
    )?;
    let _ = result;
    raisfast_derive::crud_delete!(pool, "roles", where: ("id", role_id))?;
    Ok(())
}

/// Replay undo descriptors in reverse order. Each undo failure is logged and
/// skipped (best-effort); the function itself only fails on unexpected
/// infrastructure errors, which it propagates.
pub async fn run_undo_reverse(ctx: &UndoCtx, steps: &[LogStep], keep_data: bool) -> AppResult<()> {
    for step in steps.iter().rev() {
        if let Err(e) = run_undo(ctx, &step.undo, keep_data).await {
            tracing::warn!(
                "app undo step '{}' failed (continuing best-effort): {e}",
                step.step
            );
        }
    }
    Ok(())
}

// ── Drain orchestration (§3.1 修订 #7) ──────────────────────────────

/// Outcome of the drain window.
#[derive(Debug, Default, serde::Serialize)]
pub struct DrainReport {
    pub channel_ids: Vec<i64>,
    pub drained_clean: bool,
    pub timeout_dead: usize,
}

/// Take the app's channels offline, then wait up to `window` for in-flight
/// receipts to reach a terminal state. On timeout the remainder is marked
/// dead with a `drained:timeout` step note (真实死信, never fake failures
/// from recycled routes).
pub async fn drain_channels(
    pool: &crate::db::Pool,
    plane: Option<&Arc<IntegrationPlane>>,
    channel_ids: &[i64],
    window: Duration,
) -> AppResult<DrainReport> {
    let mut report = DrainReport {
        channel_ids: channel_ids.to_vec(),
        drained_clean: true,
        timeout_dead: 0,
    };
    if channel_ids.is_empty() {
        return Ok(report);
    }

    // 1. Routes down first: channels disabled + caches refreshed.
    set_channels_enabled(pool, plane, channel_ids, false).await?;

    // 2. Drain window: poll until no non-terminal receipts remain.
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let inflight = count_inflight_receipts(pool, channel_ids).await?;
        if inflight == 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            report.timeout_dead = dead_letter_remaining(pool, channel_ids).await?;
            report.drained_clean = false;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(report)
}

async fn set_channels_enabled(
    pool: &crate::db::Pool,
    plane: Option<&Arc<IntegrationPlane>>,
    channel_ids: &[i64],
    enabled: bool,
) -> AppResult<()> {
    let placeholders = (3..=channel_ids.len() + 2)
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
    for id in channel_ids {
        q = q.bind(id);
    }
    q.execute(pool).await?;
    if let Some(plane) = plane {
        plane.channels().refresh().await?;
        plane.wake_supervisor();
    }
    Ok(())
}

async fn count_inflight_receipts(pool: &crate::db::Pool, channel_ids: &[i64]) -> AppResult<i64> {
    let placeholders = (3..=channel_ids.len() + 2)
        .map(crate::db::Driver::ph)
        .collect::<Vec<_>>();
    let sql = format!(
        "SELECT {} FROM itg_receipts WHERE status IN ({}, {}) AND channel_id IN ({})",
        crate::db::Driver::cast_int("COUNT(*)"),
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        placeholders.join(", ")
    );
    let mut q = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql))
        .bind(crate::integration::receipt::STATUS_RECEIVED)
        .bind(crate::integration::receipt::STATUS_RETRYING);
    for id in channel_ids {
        q = q.bind(id);
    }
    Ok(q.fetch_one(pool).await?)
}

/// Mark remaining non-terminal receipts dead, appending a `drained:timeout`
/// step note to each. Returns the number dead-lettered.
async fn dead_letter_remaining(pool: &crate::db::Pool, channel_ids: &[i64]) -> AppResult<usize> {
    let placeholders = (1..=channel_ids.len())
        .map(crate::db::Driver::ph)
        .collect::<Vec<_>>();
    let select = format!(
        "SELECT id, steps FROM itg_receipts WHERE status IN ('{}', '{}') AND channel_id IN ({})",
        crate::integration::receipt::STATUS_RECEIVED,
        crate::integration::receipt::STATUS_RETRYING,
        placeholders.join(", ")
    );
    let mut q = sqlx::query_as::<_, (i64, Option<Value>)>(crate::db::safe_sql(&select));
    for id in channel_ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(pool).await?;

    for (id, steps) in &rows {
        let mut steps = steps.clone().unwrap_or(Value::Array(Vec::new()));
        if let Some(arr) = steps.as_array_mut() {
            arr.push(serde_json::json!({
                "step": "app-drain",
                "status": "failed",
                "note": "drained:timeout"
            }));
        }
        let update = format!(
            "UPDATE itg_receipts SET status = {}, steps = {} WHERE id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3)
        );
        sqlx::query(crate::db::safe_sql(&update))
            .bind(crate::integration::receipt::STATUS_DEAD)
            .bind(steps)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(rows.len())
}

// ── Install-log helpers ─────────────────────────────────────────────

/// Parse the persisted install log (`apps.install_log` JSON).
#[must_use]
pub fn parse_log(log: Option<&Value>) -> Vec<LogStep> {
    log.and_then(|l| l.get("steps"))
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| serde_json::from_value(s.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Channel ids an app installed (from the compensation log).
#[must_use]
pub fn channel_ids(steps: &[LogStep]) -> Vec<i64> {
    steps
        .iter()
        .filter_map(|s| match &s.undo {
            UndoAction::DeleteChannel { id } => id.parse::<i64>().ok(),
            _ => None,
        })
        .collect()
}

/// Namespaced plugin ids an app materialized.
#[must_use]
pub fn plugin_ids(steps: &[LogStep]) -> Vec<String> {
    steps
        .iter()
        .filter_map(|s| match &s.undo {
            UndoAction::RemoveCrons { plugin_id } | UndoAction::UnloadPlugin { plugin_id } => {
                Some(plugin_id.clone())
            }
            _ => None,
        })
        .collect()
}

/// Materialized plugin directories (manifest parent dirs for enable-time
/// loading).
#[must_use]
pub fn plugin_dirs(steps: &[LogStep]) -> Vec<String> {
    steps
        .iter()
        .filter_map(|s| match &s.undo {
            UndoAction::DeletePluginDir { dir } => Some(dir.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_roundtrip_and_extraction() {
        let steps = vec![
            LogStep {
                seq: 1,
                step: "permissions".to_string(),
                detail: "role".into(),
                undo: UndoAction::DropRole {
                    role_id: "7".to_string(),
                },
                done: true,
            },
            LogStep {
                seq: 2,
                step: "channels".to_string(),
                detail: "ch".into(),
                undo: UndoAction::DeleteChannel {
                    id: "42".to_string(),
                },
                done: true,
            },
            LogStep {
                seq: 3,
                step: "crons".to_string(),
                detail: "cron".into(),
                undo: UndoAction::RemoveCrons {
                    plugin_id: "demo-app/p".into(),
                },
                done: true,
            },
        ];
        let json = serde_json::json!({ "steps": steps });
        let parsed = parse_log(Some(&json));
        assert_eq!(parsed.len(), 3);
        assert_eq!(channel_ids(&parsed), vec![42]);
        assert_eq!(plugin_ids(&parsed), vec!["demo-app/p".to_string()]);
    }
}

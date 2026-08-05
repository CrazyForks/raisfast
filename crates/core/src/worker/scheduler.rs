//! Cron scheduler
//!
//! Persistent scheduled task dispatch based on `cron_schedules` table.
//! A background loop scans for due schedules and automatically enqueues corresponding Jobs.
//!
//! # Data Flow
//!
//! ```text
//! cron_schedules table → CronScheduler (background loop) → enqueue(Job) → jobs table → WorkerRunner
//! ```
//!
//! # Cron Expression Format
//!
//! Seven-segment (including seconds): `second minute hour day month weekday year`
//!
//! ```text
//! ┌────────── second (0-59)
//! │ ┌──────── minute (0-59)
//! │ │ ┌────── hour (0-23)
//! │ │ │ ┌──── day (1-31)
//! │ │ │ │ ┌── month (1-12)
//! │ │ │ │ │ ┌ weekday (0-6, 0=Sunday)
//! │ │ │ │ │ │ ┌ year (optional)
//! │ │ │ │ │ │ │
//! * * * * * * *
//! ```
//!
//! Examples:
//! - `0 */5 * * * *` — every 5 minutes
//! - `0 0 */2 * * *` — every 2 hours
//! - `0 0 3 * * *` — daily at 3:00 AM
//! - `0 30 4 * * 1 *` — every Monday at 4:30 AM

use chrono::{DateTime, Utc};
use cron::Schedule;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::Pool;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::plugins::CronEntry;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

use super::{CronExecStatus, Job, JobQueue, NewJob};

macro_rules! cron_row_to_schedule {
    ($r:expr) => {{
        let r = $r;
        CronSchedule {
            id: r.id,
            label: r.label,
            job_type: r.job_type,
            payload: r.payload,
            cron_expr: r.cron_expr,
            enabled: r.enabled,
            last_run_at: r.last_run_at,
            next_run_at: r.next_run_at,
            plugin_id: r.plugin_id,
            exec_kind: r.exec_kind,
            handler_id: r.handler_id,
            params: r.params,
            script_lang: r.script_lang,
            script_source: r.script_source,
            script_entry: r.script_entry,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }};
}

macro_rules! exec_log_row_to_struct {
    ($r:expr) => {{
        let r = $r;
        CronExecutionLog {
            id: r.id,
            schedule_id: r.schedule_id,
            job_type: r.job_type,
            label: r.label,
            status: r.status,
            duration_ms: r.duration_ms,
            error: r.error,
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
    }};
}

#[derive(sqlx::FromRow)]
struct CronScheduleRow {
    id: SnowflakeId,
    label: String,
    job_type: String,
    payload: Option<String>,
    cron_expr: String,
    enabled: bool,
    last_run_at: Option<Timestamp>,
    next_run_at: Timestamp,
    plugin_id: Option<String>,
    exec_kind: String,
    handler_id: Option<String>,
    params: Option<String>,
    script_lang: Option<String>,
    script_source: Option<String>,
    script_entry: String,
    created_at: Timestamp,
    updated_at: Timestamp,
}

#[derive(sqlx::FromRow)]
struct CronExecLogRow {
    id: SnowflakeId,
    schedule_id: SnowflakeId,
    job_type: String,
    label: String,
    status: CronExecStatus,
    duration_ms: Option<i64>,
    error: Option<String>,
    started_at: Timestamp,
    finished_at: Option<Timestamp>,
}

#[derive(sqlx::FromRow)]
struct PluginCronRow {
    id: SnowflakeId,
    job_type: String,
}

/// Cron schedule row
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct CronSchedule {
    pub id: SnowflakeId,
    pub label: String,
    pub job_type: String,
    pub payload: Option<String>,
    pub cron_expr: String,
    pub enabled: bool,
    pub last_run_at: Option<Timestamp>,
    pub next_run_at: Timestamp,
    pub plugin_id: Option<String>,
    pub exec_kind: String,
    pub handler_id: Option<String>,
    pub params: Option<String>,
    pub script_lang: Option<String>,
    pub script_source: Option<String>,
    pub script_entry: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Compute the next execution time
///
/// Finds the next time matching the cron expression starting from `after`.
/// Uses UTC timezone.
pub fn next_run<Tz: chrono::TimeZone>(
    cron_expr: &str,
    after: chrono::DateTime<Tz>,
) -> AppResult<DateTime<Utc>> {
    let schedule = cron_expr
        .parse::<Schedule>()
        .map_err(|e| AppError::BadRequest(format!("invalid cron expression: {e}")))?;
    schedule
        .after(&after)
        .next()
        .map(|dt| dt.with_timezone(&Utc))
        .ok_or_else(|| AppError::BadRequest("cron schedule has no future runs".into()))
}

/// Create a new Cron schedule
pub async fn create_schedule(
    pool: &Pool,
    label: &str,
    job_type: &str,
    payload: Option<&str>,
    cron_expr: &str,
    enabled: bool,
) -> AppResult<CronSchedule> {
    create_schedule_with_plugin(pool, label, job_type, payload, cron_expr, enabled, None).await
}

/// Create a new Cron schedule (with `plugin_id`)
pub async fn create_schedule_with_plugin(
    pool: &Pool,
    label: &str,
    job_type: &str,
    payload: Option<&str>,
    cron_expr: &str,
    enabled: bool,
    plugin_id: Option<&str>,
) -> AppResult<CronSchedule> {
    let now = crate::utils::tz::now_utc();
    let next = next_run(cron_expr, now)?;

    let id = crate::utils::id::new_snowflake_id();
    raisfast_derive::crud_insert!(pool, "cron_schedules", [
        "id" => id,
        "label" => label,
        "job_type" => job_type,
        "payload" => payload,
        "cron_expr" => cron_expr,
        "enabled" => enabled,
        "next_run_at" => next,
        "plugin_id" => plugin_id,
        "exec_kind" => "builtin",
        "handler_id" => job_type,
        "params" => payload,
        "created_at" => now,
        "updated_at" => now
    ])?;

    Ok(find_by_id(pool, id).await?.unwrap_or(CronSchedule {
        id,
        label: label.to_string(),
        job_type: job_type.to_string(),
        payload: payload.map(|s| s.to_string()),
        cron_expr: cron_expr.to_string(),
        enabled,
        last_run_at: None,
        next_run_at: next,
        plugin_id: plugin_id.map(|s| s.to_string()),
        exec_kind: "builtin".into(),
        handler_id: Some(job_type.to_string()),
        params: payload.map(|s| s.to_string()),
        script_lang: None,
        script_source: None,
        script_entry: "on_cron_tick".to_string(),
        created_at: now,
        updated_at: now,
    }))
}

/// Create a cron schedule with full control over exec_kind / handler_id / params.
///
/// This is the preferred constructor for admin-created schedules.
pub async fn create_schedule_v2(
    pool: &Pool,
    label: &str,
    cron_expr: &str,
    enabled: bool,
    exec_kind: &str,
    handler_id: Option<&str>,
    params: Option<&str>,
) -> AppResult<CronSchedule> {
    let now = crate::utils::tz::now_utc();
    let next = next_run(cron_expr, now)?;
    let id = crate::utils::id::new_snowflake_id();

    raisfast_derive::crud_insert!(pool, "cron_schedules", [
        "id" => id,
        "label" => label,
        "job_type" => handler_id.unwrap_or(if exec_kind == "script" { "run_script" } else { "custom" }),
        "payload" => params,
        "cron_expr" => cron_expr,
        "enabled" => enabled,
        "next_run_at" => next,
        "plugin_id" => Option::<&str>::None,
        "exec_kind" => exec_kind,
        "handler_id" => handler_id,
        "params" => params,
        "created_at" => now,
        "updated_at" => now
    ])?;

    Ok(find_by_id(pool, id).await?.unwrap_or(CronSchedule {
        id,
        label: label.to_string(),
        job_type: handler_id
            .unwrap_or(if exec_kind == "script" {
                "run_script"
            } else {
                "custom"
            })
            .to_string(),
        payload: params.map(|s| s.to_string()),
        cron_expr: cron_expr.to_string(),
        enabled,
        last_run_at: None,
        next_run_at: next,
        plugin_id: None,
        exec_kind: exec_kind.to_string(),
        handler_id: handler_id.map(|s| s.to_string()),
        params: params.map(|s| s.to_string()),
        script_lang: None,
        script_source: None,
        script_entry: "on_cron_tick".to_string(),
        created_at: now,
        updated_at: now,
    }))
}

/// Create a cron schedule for a sandbox script.
///
/// `script_lang` must be `"js"`, `"lua"`, or `"rhai"`.
/// `script_source` is the raw script code.
/// `script_entry` is the function name to call (default `"on_cron_tick"`).
pub async fn create_script_schedule(
    pool: &Pool,
    label: &str,
    cron_expr: &str,
    enabled: bool,
    script_lang: &str,
    script_source: &str,
    script_entry: Option<&str>,
) -> AppResult<CronSchedule> {
    let now = crate::utils::tz::now_utc();
    let next = next_run(cron_expr, now)?;
    let id = crate::utils::id::new_snowflake_id();
    let entry = script_entry.unwrap_or("on_cron_tick");

    raisfast_derive::crud_insert!(pool, "cron_schedules", [
        "id" => id,
        "label" => label,
        "job_type" => "run_script",
        "payload" => Option::<&str>::None,
        "cron_expr" => cron_expr,
        "enabled" => enabled,
        "next_run_at" => next,
        "plugin_id" => Option::<&str>::None,
        "exec_kind" => "script",
        "handler_id" => Option::<&str>::None,
        "params" => Option::<&str>::None,
        "script_lang" => script_lang,
        "script_source" => script_source,
        "script_entry" => entry,
        "created_at" => now,
        "updated_at" => now
    ])?;

    Ok(find_by_id(pool, id).await?.unwrap_or(CronSchedule {
        id,
        label: label.to_string(),
        job_type: "run_script".to_string(),
        payload: None,
        cron_expr: cron_expr.to_string(),
        enabled,
        last_run_at: None,
        next_run_at: next,
        plugin_id: None,
        exec_kind: "script".to_string(),
        handler_id: None,
        params: None,
        script_lang: Some(script_lang.to_string()),
        script_source: Some(script_source.to_string()),
        script_entry: entry.to_string(),
        created_at: now,
        updated_at: now,
    }))
}

/// Find by ID
pub async fn find_by_id(pool: &Pool, id: SnowflakeId) -> AppResult<Option<CronSchedule>> {
    let row: Option<CronScheduleRow> = sqlx::query_as::<_, CronScheduleRow>(&format!(
        "SELECT id, label, job_type, payload, cron_expr, enabled, last_run_at, next_run_at, plugin_id, exec_kind, handler_id, params, script_lang, script_source, script_entry, created_at, updated_at
         FROM cron_schedules WHERE id = {}",
        Driver::ph(1)
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| cron_row_to_schedule!(r)))
}

/// List all schedules
pub async fn list_schedules(pool: &Pool) -> AppResult<Vec<CronSchedule>> {
    let rows: Vec<CronScheduleRow> = sqlx::query_as::<_, CronScheduleRow>(
        "SELECT id, label, job_type, payload, cron_expr, enabled, last_run_at, next_run_at, plugin_id, exec_kind, handler_id, params, script_lang, script_source, script_entry, created_at, updated_at
         FROM cron_schedules ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| cron_row_to_schedule!(r)).collect())
}

/// Enable/disable a schedule
///
/// When **enabling**, `next_run_at` is recalculated from `now` so a schedule
/// that was disabled for a long time does not fire immediately in a burst
/// (its stale `next_run_at` would be far in the past).
pub async fn toggle_schedule(pool: &Pool, id: SnowflakeId, enabled: bool) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();

    // When enabling, recompute next_run_at from cron_expr so a long-disabled
    // schedule doesn't fire-storm on re-enable.
    let next_run_at: Option<Timestamp> = if enabled {
        let row: Option<(String,)> = sqlx::query_as(&format!(
            "SELECT cron_expr FROM cron_schedules WHERE id = {}",
            Driver::ph(1)
        ))
        .bind(id)
        .fetch_optional(pool)
        .await?;

        match row {
            Some((cron_expr,)) => Some(next_run(&cron_expr, now)?),
            None => return Err(AppError::not_found("cron_schedule")),
        }
    } else {
        None
    };

    let result: crate::db::pool::DbQueryResult = raisfast_derive::crud_update!(pool, "cron_schedules",
        bind: ["enabled" => enabled, "updated_at" => now],
        optional: ["next_run_at" => next_run_at],
        where: ("id", id)
    )?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("cron_schedule"));
    }
    Ok(())
}

/// Update schedule fields
///
/// Merges provided fields, recalculates `next_run_at`, persists and returns the updated schedule.
pub async fn update_schedule(
    pool: &Pool,
    id: SnowflakeId,
    label: Option<String>,
    cron_expr: Option<String>,
    enabled: Option<bool>,
    exec_kind: Option<String>,
    handler_id: Option<String>,
    params: Option<String>,
    script_lang: Option<String>,
    script_source: Option<String>,
    script_entry: Option<String>,
) -> AppResult<CronSchedule> {
    let mut schedule = find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("cron_schedule"))?;

    if let Some(v) = label {
        schedule.label = v;
    }
    if let Some(v) = cron_expr {
        schedule.cron_expr = v;
    }
    if let Some(v) = enabled {
        schedule.enabled = v;
    }
    if let Some(v) = exec_kind {
        schedule.exec_kind = v;
    }
    if let Some(v) = handler_id {
        schedule.handler_id = Some(v);
        schedule.job_type = schedule.handler_id.clone().unwrap_or(schedule.job_type);
    }
    if let Some(v) = params {
        schedule.params = Some(v);
        schedule.payload = schedule.params.clone();
    }
    if let Some(v) = script_lang {
        schedule.script_lang = Some(v);
    }
    if let Some(v) = script_source {
        schedule.script_source = Some(v);
    }
    if let Some(v) = script_entry {
        schedule.script_entry = v;
    }

    let next = next_run(&schedule.cron_expr, crate::utils::tz::now_utc())?;
    let now = crate::utils::tz::now_utc();

    raisfast_derive::crud_update!(pool, "cron_schedules",
        bind: [
            "label" => &schedule.label,
            "job_type" => &schedule.job_type,
            "payload" => &schedule.payload,
            "cron_expr" => &schedule.cron_expr,
            "enabled" => schedule.enabled,
            "exec_kind" => &schedule.exec_kind,
            "handler_id" => &schedule.handler_id,
            "params" => &schedule.params,
            "script_lang" => &schedule.script_lang,
            "script_source" => &schedule.script_source,
            "script_entry" => &schedule.script_entry,
            "next_run_at" => next,
            "updated_at" => now
        ],
        where: ("id", id)
    )?;

    Ok(find_by_id(pool, id).await?.unwrap_or(schedule))
}

/// Delete a schedule
pub async fn delete_schedule(pool: &Pool, id: SnowflakeId) -> AppResult<()> {
    let result: crate::db::DbQueryResult =
        raisfast_derive::crud_delete!(pool, "cron_schedules", where: ("id", id))?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("cron_schedule"));
    }
    Ok(())
}

/// Cron scheduler background task
///
/// Periodically scans due `cron_schedules`, enqueues corresponding Jobs,
/// and updates `last_run_at` / `next_run_at`.
pub struct CronScheduler {
    pool: Pool,
    queue: std::sync::Arc<dyn JobQueue>,
    tick_interval: std::time::Duration,
}

impl CronScheduler {
    /// Create a new scheduler
    pub fn new(
        pool: Pool,
        queue: std::sync::Arc<dyn JobQueue>,
        tick_interval: std::time::Duration,
    ) -> Self {
        Self {
            pool,
            queue,
            tick_interval,
        }
    }

    /// Start the background scheduling loop
    pub fn spawn(self) {
        tokio::spawn(async move {
            tracing::info!("cron scheduler started (tick={:?})", self.tick_interval);
            let mut interval = tokio::time::interval(self.tick_interval);

            loop {
                interval.tick().await;
                if let Err(e) = self.tick().await {
                    tracing::error!("cron scheduler tick error: {e}");
                }
            }
        });
    }

    async fn tick(&self) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();

        let rows = sqlx::query_as::<_, CronScheduleRow>(&format!(
            "SELECT id, label, job_type, payload, cron_expr, enabled, last_run_at, next_run_at, plugin_id, exec_kind, handler_id, params, script_lang, script_source, script_entry, created_at, updated_at
             FROM cron_schedules WHERE enabled = TRUE AND next_run_at <= {}",
            Driver::ph(1)
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await?;

        for row in rows {
            let schedule = cron_row_to_schedule!(row);

            if let Err(e) = self.dispatch(&schedule).await {
                tracing::error!(
                    schedule = %schedule.label,
                    error = %e,
                    "cron dispatch failed"
                );
            }
        }

        Ok(())
    }

    async fn dispatch(&self, schedule: &CronSchedule) -> AppResult<()> {
        tracing::info!(
            "cron dispatching: {} ({})",
            schedule.label,
            schedule.job_type
        );

        // Create execution log row (status: running). The real outcome
        // (Completed/Failed/Dead) is written back by `WorkerRunner` after the
        // job finishes — NOT here. This fixes the "fake log" bug where dispatch
        // reported Completed based on enqueue success alone.
        let log_id: Option<i64> =
            create_execution_log(&self.pool, schedule.id, &schedule.job_type, &schedule.label)
                .await
                .ok();

        let job = self.build_job(schedule);
        let dispatch_result = match job {
            Ok(j) => {
                // Attach cron provenance so WorkerRunner can write back the log.
                let mut new_job = NewJob::from(j);
                new_job.cron_schedule_id = Some(schedule.id);
                new_job.cron_log_id = log_id.map(SnowflakeId);
                self.queue.enqueue(new_job).await
            }
            Err(e) => Err(e),
        };

        let now = crate::utils::tz::now_utc();
        // Use UTC consistently — `next_run` accepts any TimeZone, but mixing
        // site_tz here while `tick()` queries with UTC caused off-by-one-hour
        // drift near DST boundaries. UTC is the single source of truth.
        let next = next_run(&schedule.cron_expr, now).ok();

        in_transaction!(&self.pool, tx, {
            match &dispatch_result {
                Ok(()) => {
                    // Mark as "dispatched" (enqueued, not yet executed).
                    // WorkerRunner will update to Completed/Failed/Dead later.
                    if let Some(ref lid) = log_id {
                        raisfast_derive::crud_update!(&mut *tx, "cron_execution_log",
                            bind: ["status" => CronExecStatus::Dispatched],
                            where: ("id", lid)
                        )?;
                    }
                }
                Err(e) => {
                    // Enqueue itself failed — record immediately as Failed.
                    if let Some(ref lid) = log_id {
                        let err_str = e.to_string();
                        raisfast_derive::crud_update!(&mut *tx, "cron_execution_log",
                            bind: ["status" => CronExecStatus::Failed, "error" => &err_str, "finished_at" => now],
                            where: ("id", lid)
                        )?;
                    }
                    tracing::error!("cron dispatch failed for '{}': {e}", schedule.label);
                }
            }

            if let Some(next) = &next {
                raisfast_derive::crud_update!(&mut *tx, "cron_schedules",
                    bind: ["last_run_at" => now, "next_run_at" => next, "updated_at" => now],
                    where: ("id", schedule.id)
                )?;
            }

            Ok::<_, crate::errors::app_error::AppError>(())
        })?;

        dispatch_result
    }

    fn build_job(&self, schedule: &CronSchedule) -> AppResult<Job> {
        // For builtin exec_kind, use handler_id + params directly
        if schedule.exec_kind == "builtin" {
            let job_type = schedule.handler_id.as_deref().unwrap_or(&schedule.job_type);
            let payload: serde_json::Value = schedule
                .params
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| {
                    // Fall back to legacy payload field for backward compat
                    schedule
                        .payload
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(serde_json::Value::Null)
                });
            return Ok(Job::Custom {
                job_type: job_type.to_string(),
                payload,
            });
        }

        // For script exec_kind, dispatch to run_script handler.
        // The handler reads script_source/script_lang from cron_schedules by schedule_id.
        if schedule.exec_kind == "script" {
            let entry = schedule.script_entry.clone();
            let payload = serde_json::json!({
                "schedule_id": schedule.id.0,
                "entry": entry,
            });
            return Ok(Job::Custom {
                job_type: "run_script".to_string(),
                payload,
            });
        }

        // Legacy / plugin path: try tagged-enum round-trip first
        let tagged = match &schedule.payload {
            Some(p) if !p.is_empty() => {
                format!(r#"{{"type":"{}","payload":{}}}"#, schedule.job_type, p)
            }
            _ => format!(r#"{{"type":"{}"}}"#, schedule.job_type),
        };

        if let Ok(job) = serde_json::from_str::<Job>(&tagged) {
            return Ok(job);
        }

        let payload_value: serde_json::Value = match &schedule.payload {
            Some(p) if !p.is_empty() => serde_json::from_str(p).unwrap_or(serde_json::Value::Null),
            _ => serde_json::Value::Null,
        };

        Ok(Job::Custom {
            job_type: schedule.job_type.clone(),
            payload: payload_value,
        })
    }
}

/// Seed schedules (called on every startup)
///
/// **Idempotent**: uses `(plugin_id IS NULL, handler_id/job_type)` as the
/// natural key to decide insert-vs-skip. This ensures newly-added default
/// schedules appear on upgrade even when the table is non-empty, while
/// existing ones are left untouched (user edits preserved).
pub async fn seed_defaults(
    pool: &Pool,
    schedules: &[crate::config::app::CronScheduleConfig],
) -> AppResult<()> {
    let schedules = if schedules.is_empty() {
        crate::config::app::default_cron_schedules()
    } else {
        schedules.to_vec()
    };

    let mut inserted = 0u64;
    for s in &schedules {
        // Natural key: (plugin_id IS NULL, job_type) — only seed built-in schedules.
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM cron_schedules WHERE plugin_id IS NULL AND job_type = {})",
            Driver::ph(1)
        ))
        .bind(&s.job_type)
        .fetch_one(pool)
        .await?;

        if exists {
            continue;
        }

        create_schedule(
            pool,
            &s.label,
            &s.job_type,
            s.payload.as_deref(),
            &s.cron_expr,
            s.enabled,
        )
        .await?;
        inserted += 1;
    }

    if inserted > 0 {
        tracing::info!("seeded {inserted} new cron schedule(s)");
    }
    Ok(())
}

/// Cron execution history record
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct CronExecutionLog {
    pub id: SnowflakeId,
    pub schedule_id: SnowflakeId,
    pub job_type: String,
    pub label: String,
    pub status: CronExecStatus,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

/// Create an execution log (status: running)
pub async fn create_execution_log(
    pool: &Pool,
    schedule_id: SnowflakeId,
    job_type: &str,
    label: &str,
) -> AppResult<i64> {
    let id = crate::utils::id::new_id();
    let now = crate::utils::tz::now_utc();

    raisfast_derive::crud_insert!(pool, "cron_execution_log", [
        "id" => id,
        "schedule_id" => schedule_id,
        "job_type" => job_type,
        "label" => label,
        "status" => CronExecStatus::Running.as_str(),
        "started_at" => now
    ])?;

    Ok(id)
}

/// Mark execution log as succeeded
pub async fn complete_execution_log(
    pool: &Pool,
    log_id: SnowflakeId,
    duration_ms: i64,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "cron_execution_log",
        bind: ["status" => CronExecStatus::Completed, "duration_ms" => duration_ms, "finished_at" => now],
        where: ("id", log_id)
    )?;
    Ok(())
}

/// Mark execution log as failed
pub async fn fail_execution_log(
    pool: &Pool,
    log_id: SnowflakeId,
    duration_ms: i64,
    error: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "cron_execution_log",
        bind: ["status" => CronExecStatus::Failed, "duration_ms" => duration_ms, "error" => error, "finished_at" => now],
        where: ("id", log_id)
    )?;
    Ok(())
}

/// Mark execution log as completed (called by `WorkerRunner` after job finishes).
///
/// Accepts an explicit `now` to avoid clock skew within a batch.
pub async fn complete_execution_log_with(
    pool: &Pool,
    log_id: SnowflakeId,
    duration_ms: i64,
    now: crate::utils::tz::Timestamp,
) -> AppResult<()> {
    raisfast_derive::crud_update!(pool, "cron_execution_log",
        bind: ["status" => CronExecStatus::Completed, "duration_ms" => duration_ms, "finished_at" => now],
        where: ("id", log_id)
    )?;
    Ok(())
}

/// Mark execution log as failed/dead (called by `WorkerRunner` after job fails).
///
/// `status` may be `Failed` (will retry) or `Dead` (exhausted retries).
pub async fn fail_execution_log_with(
    pool: &Pool,
    log_id: SnowflakeId,
    status: crate::worker::CronExecStatus,
    error: &str,
    now: crate::utils::tz::Timestamp,
) -> AppResult<()> {
    raisfast_derive::crud_update!(pool, "cron_execution_log",
        bind: ["status" => status, "error" => error, "finished_at" => now],
        where: ("id", log_id)
    )?;
    Ok(())
}

/// Query execution history for a schedule
pub async fn list_execution_logs(
    pool: &Pool,
    schedule_id: SnowflakeId,
    limit: i64,
    offset: i64,
) -> AppResult<(Vec<CronExecutionLog>, i64)> {
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM cron_execution_log WHERE schedule_id = {}",
        Driver::ph(1)
    ))
    .bind(schedule_id)
    .fetch_one(pool)
    .await?;

    let rows: Vec<CronExecLogRow> = sqlx::query_as::<_, CronExecLogRow>(&format!(
        "SELECT el.id, el.schedule_id, el.job_type, el.label, el.status, el.duration_ms, el.error, el.started_at, el.finished_at
         FROM cron_execution_log el
         WHERE el.schedule_id = {}
         ORDER BY el.started_at DESC LIMIT {} OFFSET {}",
        Driver::ph(1), Driver::ph(2), Driver::ph(3)
    ))
    .bind(schedule_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let logs = rows
        .into_iter()
        .map(|r| exec_log_row_to_struct!(r))
        .collect();
    Ok((logs, total))
}

/// Query recent execution records across all schedules
pub async fn recent_execution_logs(pool: &Pool, limit: i64) -> AppResult<Vec<CronExecutionLog>> {
    let rows: Vec<CronExecLogRow> = sqlx::query_as::<_, CronExecLogRow>(&format!(
        "SELECT id, schedule_id, job_type, label, status, duration_ms, error, started_at, finished_at
         FROM cron_execution_log
         ORDER BY started_at DESC LIMIT {}",
        Driver::ph(1)
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| exec_log_row_to_struct!(r))
        .collect())
}

/// Clean up expired execution logs
pub async fn cleanup_execution_logs(pool: &Pool, retention_days: i64) -> AppResult<u64> {
    let threshold = crate::utils::tz::now_utc() - chrono::Duration::days(retention_days);
    let result: crate::db::pool::DbQueryResult = sqlx::query(&format!(
        "DELETE FROM cron_execution_log WHERE started_at < {}",
        Driver::ph(1)
    ))
    .bind(threshold)
    .execute(pool)
    .await?;

    let count = result.rows_affected();
    if count > 0 {
        tracing::info!("cleaned up {count} old cron execution logs");
    }
    Ok(count)
}

/// Sync plugin Cron schedules to the database
///
/// Called when plugins are loaded/reloaded. Writes `plugin.cron` declarations into `cron_schedules`,
/// and removes stale schedule entries that the plugin no longer declares.
/// All operations are performed in a single transaction for atomicity.
///
/// Uses the `plugin_id` column for association; does not affect built-in or other plugins' schedules.
pub async fn sync_plugin_crons(
    pool: &Pool,
    plugin_id: &str,
    entries: &[CronEntry],
) -> AppResult<()> {
    in_transaction!(pool, tx, {
        let old = sqlx::query_as::<_, PluginCronRow>(&format!(
            "SELECT id, job_type FROM cron_schedules WHERE plugin_id = {}",
            Driver::ph(1)
        ))
        .bind(plugin_id)
        .fetch_all(&mut *tx)
        .await?;

        let new_types: Vec<&str> = entries.iter().map(|e| e.job_type.as_str()).collect();

        for row in &old {
            if !new_types.contains(&row.job_type.as_str()) {
                raisfast_derive::crud_delete!(&mut *tx, "cron_schedules", where: ("id", row.id))?;
                tracing::info!(
                    "removed stale cron '{}' for plugin {plugin_id}",
                    row.job_type
                );
            }
        }

        for entry in entries {
            let existing: Option<(i64,)> = sqlx::query_as(&format!(
                "SELECT id FROM cron_schedules WHERE plugin_id = {} AND job_type = {}",
                Driver::ph(1),
                Driver::ph(2)
            ))
            .bind(plugin_id)
            .bind(&entry.job_type)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(existing_row) = existing {
                let now = crate::utils::tz::now_utc();
                let next = next_run(&entry.cron_expr, crate::utils::tz::now_utc())?;
                raisfast_derive::crud_update!(&mut *tx, "cron_schedules",
                    bind: ["label" => &entry.label, "payload" => &entry.payload, "cron_expr" => &entry.cron_expr, "enabled" => entry.enabled, "next_run_at" => next, "updated_at" => now],
                    where: ("id", existing_row.0)
                )?;

                tracing::debug!("updated cron '{}' for plugin {plugin_id}", entry.job_type);
            } else {
                let id = crate::utils::id::new_id();
                let now = crate::utils::tz::now_utc();
                let next = next_run(&entry.cron_expr, crate::utils::tz::now_utc())?;
                raisfast_derive::crud_insert!(&mut *tx, "cron_schedules", [
                    "id" => id,
                    "label" => &entry.label,
                    "job_type" => &entry.job_type,
                    "payload" => entry.payload.as_deref(),
                    "cron_expr" => &entry.cron_expr,
                    "enabled" => entry.enabled,
                    "next_run_at" => next,
                    "plugin_id" => plugin_id,
                    "exec_kind" => "plugin",
                    "handler_id" => Option::<&str>::None,
                    "params" => entry.payload.as_deref(),
                    "created_at" => now,
                    "updated_at" => now
                ])?;

                tracing::info!("created cron '{}' for plugin {plugin_id}", entry.job_type);
            }
        }

        Ok::<_, crate::errors::app_error::AppError>(())
    })
}

/// Remove all Cron schedules associated with a plugin
///
/// Called when a plugin is unloaded.
pub async fn remove_plugin_crons(pool: &Pool, plugin_id: &str) -> AppResult<()> {
    let result: crate::db::DbQueryResult =
        raisfast_derive::crud_delete!(pool, "cron_schedules", where: ("plugin_id", plugin_id))?;

    let count = result.rows_affected();
    if count > 0 {
        tracing::info!("removed {count} cron schedule(s) for plugin {plugin_id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_run_every_5_min() {
        let after = "2025-06-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let next = next_run("0 */5 * * * *", after).unwrap();
        assert_eq!(next.format("%H:%M").to_string(), "12:05");
    }

    #[test]
    fn next_run_daily_3am() {
        let after = "2025-06-15T14:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let next = next_run("0 0 3 * * *", after).unwrap();
        assert_eq!(next.format("%d %H:%M").to_string(), "16 03:00");
    }

    #[test]
    fn next_run_invalid_expr() {
        let after = Utc::now();
        assert!(next_run("invalid", after).is_err());
    }

    #[tokio::test]
    async fn create_and_find_schedule() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let s = create_schedule(
            &pool,
            "Test Job",
            "generate_sitemap",
            None,
            "0 0 */6 * * *",
            true,
        )
        .await
        .unwrap();

        assert_eq!(s.label, "Test Job");
        assert!(s.enabled);
        assert!(s.next_run_at.to_rfc3339().len() > 10);

        let found = find_by_id(&pool, s.id).await.unwrap().unwrap();
        assert_eq!(found.job_type, "generate_sitemap");
    }

    #[tokio::test]
    async fn toggle_and_delete_schedule() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let s = create_schedule(
            &pool,
            "Test",
            "generate_sitemap",
            None,
            "0 0 */6 * * *",
            true,
        )
        .await
        .unwrap();

        toggle_schedule(&pool, s.id, false).await.unwrap();
        let found = find_by_id(&pool, s.id).await.unwrap().unwrap();
        assert!(!found.enabled);

        delete_schedule(&pool, s.id).await.unwrap();
        assert!(find_by_id(&pool, s.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn toggle_nonexistent_returns_not_found() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let result = toggle_schedule(&pool, SnowflakeId(9999999), true).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_schedules_returns_all() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        create_schedule(&pool, "A", "generate_sitemap", None, "0 0 * * * *", true)
            .await
            .unwrap();
        create_schedule(&pool, "B", "generate_sitemap", None, "0 0 */2 * * *", true)
            .await
            .unwrap();

        let list = list_schedules(&pool).await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn seed_defaults_inserts_when_empty() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        seed_defaults(&pool, &[]).await.unwrap();

        let list = list_schedules(&pool).await.unwrap();
        assert_eq!(list.len(), 7);

        // Second call does not insert duplicates
        seed_defaults(&pool, &[]).await.unwrap();
        let list2 = list_schedules(&pool).await.unwrap();
        assert_eq!(list2.len(), 7);
    }

    #[tokio::test]
    async fn scheduler_dispatches_due_schedule() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let queue = std::sync::Arc::new(super::super::DefaultJobQueue::new(pool.clone()));

        // Manually insert a schedule with next_run_at in the past
        let now = Utc::now();
        let past = (now - chrono::Duration::hours(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO cron_schedules (id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)",
        )
        .bind(1i64)
        .bind("Test Sitemap")
        .bind("generate_sitemap")
        .bind(Option::<String>::None)
        .bind("0 0 */6 * * *")
        .bind(&past)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let scheduler = CronScheduler::new(pool.clone(), queue, std::time::Duration::from_secs(60));
        scheduler.tick().await.unwrap();

        let jobs = sqlx::query_as::<_, (String, String)>("SELECT job_type, status FROM jobs")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].0, "generate_sitemap");

        let row: (String,) = sqlx::query_as("SELECT next_run_at FROM cron_schedules WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_ne!(row.0, past);
    }

    #[tokio::test]
    async fn scheduler_skips_future_schedule() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let queue = std::sync::Arc::new(super::super::DefaultJobQueue::new(pool.clone()));

        let now = Utc::now();
        let future = (now + chrono::Duration::hours(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO cron_schedules (id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)",
        )
        .bind(2i64)
        .bind("Future Job")
        .bind("generate_sitemap")
        .bind(Option::<String>::None)
        .bind("0 0 */6 * * *")
        .bind(&future)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let scheduler = CronScheduler::new(pool.clone(), queue, std::time::Duration::from_secs(60));
        scheduler.tick().await.unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn sync_plugin_crons_creates_entries() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let entries = vec![CronEntry {
            label: "Cleanup".into(),
            job_type: "cleanup_sessions".into(),
            payload: Some(r#"{"max_age": 24}"#.into()),
            cron_expr: "0 0 */6 * * *".into(),
            enabled: true,
        }];

        sync_plugin_crons(&pool, "com.example.cleanup", &entries)
            .await
            .unwrap();

        let list = list_schedules(&pool).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].job_type, "cleanup_sessions");
        assert_eq!(list[0].plugin_id, Some("com.example.cleanup".into()));
        assert!(list[0].enabled);
    }

    #[tokio::test]
    async fn sync_plugin_crons_updates_existing() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let entries = vec![CronEntry {
            label: "V1".into(),
            job_type: "my_task".into(),
            payload: None,
            cron_expr: "0 0 * * * *".into(),
            enabled: true,
        }];
        sync_plugin_crons(&pool, "com.test", &entries)
            .await
            .unwrap();

        let updated = vec![CronEntry {
            label: "V2".into(),
            job_type: "my_task".into(),
            payload: None,
            cron_expr: "0 0 */2 * * *".into(),
            enabled: false,
        }];
        sync_plugin_crons(&pool, "com.test", &updated)
            .await
            .unwrap();

        let list = list_schedules(&pool).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "V2");
        assert!(!list[0].enabled);
    }

    #[tokio::test]
    async fn sync_plugin_crons_removes_stale_entries() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let v1 = vec![
            CronEntry {
                label: "A".into(),
                job_type: "task_a".into(),
                payload: None,
                cron_expr: "0 0 * * * *".into(),
                enabled: true,
            },
            CronEntry {
                label: "B".into(),
                job_type: "task_b".into(),
                payload: None,
                cron_expr: "0 0 * * * *".into(),
                enabled: true,
            },
        ];
        sync_plugin_crons(&pool, "com.test", &v1).await.unwrap();
        assert_eq!(list_schedules(&pool).await.unwrap().len(), 2);

        let v2 = vec![CronEntry {
            label: "A".into(),
            job_type: "task_a".into(),
            payload: None,
            cron_expr: "0 0 * * * *".into(),
            enabled: true,
        }];
        sync_plugin_crons(&pool, "com.test", &v2).await.unwrap();

        let list = list_schedules(&pool).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].job_type, "task_a");
    }

    #[tokio::test]
    async fn remove_plugin_crons_deletes_all() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let entries = vec![CronEntry {
            label: "X".into(),
            job_type: "task_x".into(),
            payload: None,
            cron_expr: "0 0 * * * *".into(),
            enabled: true,
        }];
        sync_plugin_crons(&pool, "com.test", &entries)
            .await
            .unwrap();
        assert_eq!(list_schedules(&pool).await.unwrap().len(), 1);

        remove_plugin_crons(&pool, "com.test").await.unwrap();
        assert!(list_schedules(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_plugin_crons_does_not_affect_others() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let entries = vec![CronEntry {
            label: "X".into(),
            job_type: "task_x".into(),
            payload: None,
            cron_expr: "0 0 * * * *".into(),
            enabled: true,
        }];
        sync_plugin_crons(&pool, "com.test", &entries)
            .await
            .unwrap();

        create_schedule(
            &pool,
            "Built-in",
            "generate_sitemap",
            None,
            "0 0 * * * *",
            true,
        )
        .await
        .unwrap();

        remove_plugin_crons(&pool, "com.test").await.unwrap();
        let list = list_schedules(&pool).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].plugin_id.is_none());
    }

    async fn setup_log_tables() -> Pool {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn insert_test_schedule(pool: &Pool) -> i64 {
        let id = crate::utils::id::new_id();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO cron_schedules (id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, 'Test', 'test_task', NULL, '0 */5 * * * *', 1, ?, NULL, ?, ?)",
        )
        .bind(id)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn execution_log_create_and_complete() {
        let pool = setup_log_tables().await;
        let sched_id = insert_test_schedule(&pool).await;

        let log_id =
            create_execution_log(&pool, SnowflakeId(sched_id), "generate_sitemap", "Sitemap")
                .await
                .unwrap();

        let logs = list_execution_logs(&pool, SnowflakeId(sched_id), 10)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, CronExecStatus::Running);
        assert_eq!(logs[0].job_type, "generate_sitemap");
        assert!(logs[0].duration_ms.is_none());
        assert!(logs[0].finished_at.is_none());

        complete_execution_log(&pool, SnowflakeId(log_id), 42)
            .await
            .unwrap();

        let logs = list_execution_logs(&pool, SnowflakeId(sched_id), 10)
            .await
            .unwrap();
        assert_eq!(logs[0].status, CronExecStatus::Completed);
        assert_eq!(logs[0].duration_ms, Some(42));
        assert!(logs[0].finished_at.is_some());
    }

    #[tokio::test]
    async fn execution_log_fail_records_error() {
        let pool = setup_log_tables().await;
        let sched_id = insert_test_schedule(&pool).await;

        let log_id = create_execution_log(&pool, SnowflakeId(sched_id), "my_task", "Task")
            .await
            .unwrap();

        fail_execution_log(&pool, SnowflakeId(log_id), 100, "something broke")
            .await
            .unwrap();

        let logs = list_execution_logs(&pool, SnowflakeId(sched_id), 10)
            .await
            .unwrap();
        assert_eq!(logs[0].status, CronExecStatus::Failed);
        assert_eq!(logs[0].duration_ms, Some(100));
        assert_eq!(logs[0].error, Some("something broke".into()));
    }

    #[tokio::test]
    async fn execution_log_list_by_schedule() {
        let pool = setup_log_tables().await;
        let sched_a = insert_test_schedule(&pool).await;
        let sched_b = insert_test_schedule(&pool).await;

        create_execution_log(&pool, SnowflakeId(sched_a), "task_a", "A")
            .await
            .unwrap();
        create_execution_log(&pool, SnowflakeId(sched_b), "task_b", "B")
            .await
            .unwrap();
        create_execution_log(&pool, SnowflakeId(sched_a), "task_a", "A2")
            .await
            .unwrap();

        let a = list_execution_logs(&pool, SnowflakeId(sched_a), 10)
            .await
            .unwrap();
        assert_eq!(a.len(), 2);

        let b = list_execution_logs(&pool, SnowflakeId(sched_b), 10)
            .await
            .unwrap();
        assert_eq!(b.len(), 1);
    }

    #[tokio::test]
    async fn execution_log_recent_ordering() {
        let pool = setup_log_tables().await;
        let s1 = insert_test_schedule(&pool).await;
        let s2 = insert_test_schedule(&pool).await;

        create_execution_log(&pool, SnowflakeId(s1), "task_1", "First")
            .await
            .unwrap();
        create_execution_log(&pool, SnowflakeId(s2), "task_2", "Second")
            .await
            .unwrap();

        let recent = recent_execution_logs(&pool, 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].label, "Second");
    }

    #[tokio::test]
    async fn execution_log_cleanup_removes_old() {
        let pool = setup_log_tables().await;
        let s1 = insert_test_schedule(&pool).await;

        create_execution_log(&pool, SnowflakeId(s1), "task_1", "Old")
            .await
            .unwrap();

        let count = cleanup_execution_logs(&pool, 0).await.unwrap();
        assert_eq!(count, 1);

        let logs = list_execution_logs(&pool, SnowflakeId(s1), 10)
            .await
            .unwrap();
        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn scheduler_dispatch_creates_execution_log() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let queue = std::sync::Arc::new(super::super::DefaultJobQueue::new(pool.clone()));
        let scheduler = CronScheduler::new(pool.clone(), queue, std::time::Duration::from_secs(60));

        let now = Utc::now();
        let past = (now - chrono::Duration::hours(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO cron_schedules (id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)",
        )
        .bind(3i64)
        .bind("Log Test")
        .bind("generate_sitemap")
        .bind(Option::<String>::None)
        .bind("0 0 */6 * * *")
        .bind(&past)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        scheduler.tick().await.unwrap();

        let logs = list_execution_logs(&pool, SnowflakeId(3), 10)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, CronExecStatus::Completed);
        assert!(logs[0].duration_ms.is_some());
        assert!(logs[0].finished_at.is_some());
    }
}

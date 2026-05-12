//! Cron 定时调度器
//!
//! 基于 `cron_schedules` 表持久化的定时任务调度。
//! 后台循环扫描到期 schedule，自动入队对应 Job。
//!
//! # 数据流
//!
//! ```text
//! cron_schedules 表 → CronScheduler (后台循环) → enqueue(Job) → jobs 表 → WorkerRunner
//! ```
//!
//! # Cron 表达式格式
//!
//! 七段式（含秒）：`秒 分 时 日 月 星期 年`
//!
//! ```text
//! ┌────────── 秒 (0-59)
//! │ ┌──────── 分 (0-59)
//! │ │ ┌────── 时 (0-23)
//! │ │ │ ┌──── 日 (1-31)
//! │ │ │ │ ┌── 月 (1-12)
//! │ │ │ │ │ ┌ 星期 (0-6, 0=周日)
//! │ │ │ │ │ │ ┌ 年 (可选)
//! │ │ │ │ │ │ │
//! * * * * * * *
//! ```
//!
//! 示例：
//! - `0 */5 * * * *` — 每 5 分钟
//! - `0 0 */2 * * *` — 每 2 小时
//! - `0 0 3 * * *` — 每天凌晨 3 点
//! - `0 30 4 * * 1 *` — 每周一凌晨 4:30

use chrono::{DateTime, Utc};
use cron::Schedule;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::Pool;
use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::plugins::CronEntry;
use crate::utils::tz::Timestamp;

use super::{CronExecStatus, Job, JobQueue, NewJob};

macro_rules! cron_row_to_schedule {
    ($r:expr) => {{
        let r = $r;
        CronSchedule {
            id: r.id,
            document_id: r.document_id,
            label: r.label,
            job_type: r.job_type,
            payload: r.payload,
            cron_expr: r.cron_expr,
            enabled: r.enabled != 0,
            last_run_at: r.last_run_at,
            next_run_at: r.next_run_at,
            plugin_id: r.plugin_id,
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
            document_id: r.document_id,
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
    id: i64,
    document_id: String,
    label: String,
    job_type: String,
    payload: Option<String>,
    cron_expr: String,
    enabled: i64,
    last_run_at: Option<Timestamp>,
    next_run_at: Timestamp,
    plugin_id: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

#[derive(sqlx::FromRow)]
struct CronExecLogRow {
    id: i64,
    document_id: String,
    schedule_id: i64,
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
    id: i64,
    job_type: String,
}

/// Cron 调度行
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct CronSchedule {
    pub id: i64,
    pub document_id: String,
    pub label: String,
    pub job_type: String,
    pub payload: Option<String>,
    pub cron_expr: String,
    pub enabled: bool,
    pub last_run_at: Option<Timestamp>,
    pub next_run_at: Timestamp,
    pub plugin_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 计算下一个执行时间
///
/// 从 `after` 开始查找下一个匹配 cron 表达式的时间点。
/// 使用 UTC 时区。
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

/// 创建新的 Cron 调度
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

/// 创建新的 Cron 调度（带 `plugin_id`）
pub async fn create_schedule_with_plugin(
    pool: &Pool,
    label: &str,
    job_type: &str,
    payload: Option<&str>,
    cron_expr: &str,
    enabled: bool,
    plugin_id: Option<&str>,
) -> AppResult<CronSchedule> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_utc();
    let next = next_run(cron_expr, now)?;

    let (int_id,): (i64,) = sqlx::query_as(&format!(
        "INSERT INTO cron_schedules (document_id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         RETURNING id",
        ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10)
    ))
    .bind(&document_id)
    .bind(label)
    .bind(job_type)
    .bind(payload)
    .bind(cron_expr)
    .bind(enabled)
    .bind(next)
    .bind(plugin_id)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(CronSchedule {
        id: int_id,
        document_id,
        label: label.to_string(),
        job_type: job_type.to_string(),
        payload: payload.map(|s| s.to_string()),
        cron_expr: cron_expr.to_string(),
        enabled,
        last_run_at: None,
        next_run_at: next,
        plugin_id: plugin_id.map(|s| s.to_string()),
        created_at: now,
        updated_at: now,
    })
}

/// 按 ID 查找
pub async fn find_by_id(pool: &Pool, id: &str) -> AppResult<Option<CronSchedule>> {
    let row = sqlx::query_as::<_, CronScheduleRow>(&format!(
        "SELECT id, document_id, label, job_type, payload, cron_expr, enabled, last_run_at, next_run_at, plugin_id, created_at, updated_at
         FROM cron_schedules WHERE document_id = {}",
        ph(1)
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| cron_row_to_schedule!(r)))
}

/// 列出所有调度
pub async fn list_schedules(pool: &Pool) -> AppResult<Vec<CronSchedule>> {
    let rows = sqlx::query_as::<_, CronScheduleRow>(
        "SELECT id, document_id, label, job_type, payload, cron_expr, enabled, last_run_at, next_run_at, plugin_id, created_at, updated_at
         FROM cron_schedules ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| cron_row_to_schedule!(r)).collect())
}

/// 启用/禁用调度
pub async fn toggle_schedule(pool: &Pool, id: &str, enabled: bool) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = sqlx::query(&format!(
        "UPDATE cron_schedules SET enabled = {}, updated_at = {} WHERE document_id = {}",
        ph(1),
        ph(2),
        ph(3)
    ))
    .bind(enabled)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("cron_schedule"));
    }
    Ok(())
}

/// 更新调度字段
///
/// 合并提供的字段，重新计算 `next_run_at`，持久化并返回更新后的调度。
pub async fn update_schedule(
    pool: &Pool,
    id: &str,
    label: Option<String>,
    job_type: Option<String>,
    payload: Option<Option<String>>,
    cron_expr: Option<String>,
    enabled: Option<bool>,
) -> AppResult<CronSchedule> {
    let mut schedule = find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("cron_schedule"))?;

    if let Some(v) = label {
        schedule.label = v;
    }
    if let Some(v) = job_type {
        schedule.job_type = v;
    }
    if let Some(v) = payload {
        schedule.payload = v;
    }
    if let Some(v) = cron_expr {
        schedule.cron_expr = v;
    }
    if let Some(v) = enabled {
        schedule.enabled = v;
    }

    let next = next_run(&schedule.cron_expr, crate::utils::tz::now_utc())?;
    let now = crate::utils::tz::now_utc();

    sqlx::query(&format!(
        "UPDATE cron_schedules SET label = {}, job_type = {}, payload = {}, cron_expr = {}, enabled = {}, next_run_at = {}, updated_at = {} WHERE document_id = {}",
        ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8)
    ))
    .bind(&schedule.label)
    .bind(&schedule.job_type)
    .bind(&schedule.payload)
    .bind(&schedule.cron_expr)
    .bind(schedule.enabled)
    .bind(next)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(find_by_id(pool, id).await?.unwrap_or(schedule))
}

/// 删除调度
pub async fn delete_schedule(pool: &Pool, id: &str) -> AppResult<()> {
    let result = sqlx::query(&format!(
        "DELETE FROM cron_schedules WHERE document_id = {}",
        ph(1)
    ))
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("cron_schedule"));
    }
    Ok(())
}

/// Cron 调度器后台任务
///
/// 循环扫描到期的 `cron_schedules`，入队对应 Job，
/// 并更新 `last_run_at` / `next_run_at`。
pub struct CronScheduler {
    pool: Pool,
    queue: std::sync::Arc<dyn JobQueue>,
    tick_interval: std::time::Duration,
}

impl CronScheduler {
    /// 创建新的调度器
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

    /// 启动后台调度循环
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
            "SELECT id, document_id, label, job_type, payload, cron_expr, enabled, last_run_at, next_run_at, plugin_id, created_at, updated_at
             FROM cron_schedules WHERE enabled = 1 AND next_run_at <= {}",
            ph(1)
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

        let log_id =
            create_execution_log(&self.pool, schedule.id, &schedule.job_type, &schedule.label)
                .await
                .ok();

        let start = std::time::Instant::now();

        let job = self.build_job(schedule);
        let dispatch_result = match job {
            Ok(j) => self.queue.enqueue(NewJob::from(j)).await,
            Err(e) => Err(e),
        };

        let elapsed = start.elapsed().as_millis() as i64;

        let now = crate::utils::tz::now_utc();
        let local_now = now.with_timezone(&crate::utils::tz::site_tz());
        let next = next_run(&schedule.cron_expr, local_now).ok();

        in_transaction!(&self.pool, tx, {
            match &dispatch_result {
                Ok(()) => {
                    if let Some(ref lid) = log_id {
                        sqlx::query(&format!(
                            "UPDATE cron_execution_log SET status = 'completed', duration_ms = {}, finished_at = {} WHERE document_id = {}",
                            ph(1), ph(2), ph(3)
                        ))
                        .bind(elapsed)
                        .bind(now)
                        .bind(lid)
                        .execute(&mut *tx)
                        .await?;
                    }
                }
                Err(e) => {
                    if let Some(ref lid) = log_id {
                        let err_str = e.to_string();
                        sqlx::query(&format!(
                            "UPDATE cron_execution_log SET status = 'failed', duration_ms = {}, error = {}, finished_at = {} WHERE document_id = {}",
                            ph(1), ph(2), ph(3), ph(4)
                        ))
                        .bind(elapsed)
                        .bind(&err_str)
                        .bind(now)
                        .bind(lid)
                        .execute(&mut *tx)
                        .await?;
                    }
                    tracing::error!("cron dispatch failed for '{}': {e}", schedule.label);
                }
            }

            if let Some(next) = &next {
                sqlx::query(&format!(
                    "UPDATE cron_schedules SET last_run_at = {}, next_run_at = {}, updated_at = {} WHERE id = {}",
                    ph(1),
                    ph(2),
                    ph(3),
                    ph(4)
                ))
                .bind(now)
                .bind(next)
                .bind(now)
                .bind(schedule.id)
                .execute(&mut *tx)
                .await?;
            }

            Ok::<_, crate::errors::app_error::AppError>(())
        })?;

        dispatch_result
    }

    fn build_job(&self, schedule: &CronSchedule) -> AppResult<Job> {
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

/// 插入调度（首次启动时调用）
///
/// 在 `cron_schedules` 表为空时，从 `AppConfig.cron_schedules` 插入。
/// 若配置为空则使用 `default_cron_schedules()` 内置默认值。
pub async fn seed_defaults(
    pool: &Pool,
    schedules: &[crate::config::app::CronScheduleConfig],
) -> AppResult<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cron_schedules")
        .fetch_one(pool)
        .await?;

    if count > 0 {
        return Ok(());
    }

    let schedules = if schedules.is_empty() {
        crate::config::app::default_cron_schedules()
    } else {
        schedules.to_vec()
    };

    tracing::info!("seeding {} cron schedule(s)", schedules.len());

    for s in &schedules {
        create_schedule(
            pool,
            &s.label,
            &s.job_type,
            s.payload.as_deref(),
            &s.cron_expr,
            s.enabled,
        )
        .await?;
    }

    Ok(())
}

/// Cron 执行历史记录
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct CronExecutionLog {
    pub id: i64,
    pub document_id: String,
    pub schedule_id: i64,
    pub job_type: String,
    pub label: String,
    pub status: CronExecStatus,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

/// 创建执行日志（状态 running）
pub async fn create_execution_log(
    pool: &Pool,
    schedule_id: i64,
    job_type: &str,
    label: &str,
) -> AppResult<String> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_utc();

    sqlx::query(&format!(
        "INSERT INTO cron_execution_log (document_id, schedule_id, job_type, label, status, started_at)
         VALUES ({}, {}, {}, {}, 'running', {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5)
    ))
    .bind(&document_id)
    .bind(schedule_id)
    .bind(job_type)
    .bind(label)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(document_id)
}

/// 标记执行日志为成功
pub async fn complete_execution_log(pool: &Pool, log_id: &str, duration_ms: i64) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    sqlx::query(&format!(
        "UPDATE cron_execution_log SET status = 'completed', duration_ms = {}, finished_at = {} WHERE document_id = {}",
        ph(1), ph(2), ph(3)
    ))
    .bind(duration_ms)
    .bind(now)
    .bind(log_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 标记执行日志为失败
pub async fn fail_execution_log(
    pool: &Pool,
    log_id: &str,
    duration_ms: i64,
    error: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    sqlx::query(&format!(
        "UPDATE cron_execution_log SET status = 'failed', duration_ms = {}, error = {}, finished_at = {} WHERE document_id = {}",
        ph(1), ph(2), ph(3), ph(4)
    ))
    .bind(duration_ms)
    .bind(error)
    .bind(now)
    .bind(log_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 查询某个 schedule 的执行历史
pub async fn list_execution_logs(
    pool: &Pool,
    schedule_document_id: &str,
    limit: i64,
) -> AppResult<Vec<CronExecutionLog>> {
    let rows = sqlx::query_as::<_, CronExecLogRow>(&format!(
        "SELECT el.id, el.document_id, el.schedule_id, el.job_type, el.label, el.status, el.duration_ms, el.error, el.started_at, el.finished_at
         FROM cron_execution_log el
         JOIN cron_schedules cs ON el.schedule_id = cs.id
         WHERE cs.document_id = {}
         ORDER BY el.started_at DESC LIMIT {}",
        ph(1), ph(2)
    ))
    .bind(schedule_document_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| exec_log_row_to_struct!(r))
        .collect())
}

/// 查询所有 schedule 的最近执行记录
pub async fn recent_execution_logs(pool: &Pool, limit: i64) -> AppResult<Vec<CronExecutionLog>> {
    let rows = sqlx::query_as::<_, CronExecLogRow>(&format!(
        "SELECT id, document_id, schedule_id, job_type, label, status, duration_ms, error, started_at, finished_at
         FROM cron_execution_log
         ORDER BY started_at DESC LIMIT {}",
        ph(1)
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| exec_log_row_to_struct!(r))
        .collect())
}

/// 清理过期的执行日志
pub async fn cleanup_execution_logs(pool: &Pool, retention_days: i64) -> AppResult<u64> {
    let threshold = crate::utils::tz::now_utc() - chrono::Duration::days(retention_days);
    let result = sqlx::query(&format!(
        "DELETE FROM cron_execution_log WHERE started_at < {}",
        ph(1)
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

/// 同步插件的 Cron 调度到数据库
///
/// 插件加载/重载时调用。将 `plugin.cron` 声明写入 `cron_schedules`，
/// 同时删除该插件之前声明但已不存在的调度条目。
/// 全部操作在单个事务中完成，确保原子性。
///
/// 使用 `plugin_id` 列关联，不影响内置调度或其他插件的调度。
pub async fn sync_plugin_crons(
    pool: &Pool,
    plugin_id: &str,
    entries: &[CronEntry],
) -> AppResult<()> {
    in_transaction!(pool, tx, {
        let old = sqlx::query_as::<_, PluginCronRow>(&format!(
            "SELECT id, job_type FROM cron_schedules WHERE plugin_id = {}",
            ph(1)
        ))
        .bind(plugin_id)
        .fetch_all(&mut *tx)
        .await?;

        let new_types: Vec<&str> = entries.iter().map(|e| e.job_type.as_str()).collect();

        for row in &old {
            if !new_types.contains(&row.job_type.as_str()) {
                sqlx::query(&format!("DELETE FROM cron_schedules WHERE id = {}", ph(1)))
                    .bind(row.id)
                    .execute(&mut *tx)
                    .await?;
                tracing::info!(
                    "removed stale cron '{}' for plugin {plugin_id}",
                    row.job_type
                );
            }
        }

        for entry in entries {
            let existing: Option<(i64,)> = sqlx::query_as(&format!(
                "SELECT id FROM cron_schedules WHERE plugin_id = {} AND job_type = {}",
                ph(1),
                ph(2)
            ))
            .bind(plugin_id)
            .bind(&entry.job_type)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(existing_row) = existing {
                let now = crate::utils::tz::now_utc();
                let next = next_run(&entry.cron_expr, crate::utils::tz::now_utc())?;
                sqlx::query(&format!(
                    "UPDATE cron_schedules SET label = {}, payload = {}, cron_expr = {}, enabled = {}, next_run_at = {}, updated_at = {} WHERE id = {}",
                    ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7)
                ))
                .bind(&entry.label)
                .bind(&entry.payload)
                .bind(&entry.cron_expr)
                .bind(entry.enabled)
                .bind(next)
                .bind(now)
                .bind(existing_row.0)
                .execute(&mut *tx)
                .await?;

                tracing::debug!("updated cron '{}' for plugin {plugin_id}", entry.job_type);
            } else {
                let document_id = uuid::Uuid::now_v7().to_string();
                let now = crate::utils::tz::now_utc();
                let next = next_run(&entry.cron_expr, crate::utils::tz::now_utc())?;
                sqlx::query(&format!(
                    "INSERT INTO cron_schedules (document_id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10)
                ))
                .bind(&document_id)
                .bind(&entry.label)
                .bind(&entry.job_type)
                .bind(&entry.payload)
                .bind(&entry.cron_expr)
                .bind(entry.enabled)
                .bind(next)
                .bind(plugin_id)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;

                tracing::info!("created cron '{}' for plugin {plugin_id}", entry.job_type);
            }
        }

        Ok::<_, crate::errors::app_error::AppError>(())
    })
}

/// 删除插件关联的所有 Cron 调度
///
/// 插件卸载时调用。
pub async fn remove_plugin_crons(pool: &Pool, plugin_id: &str) -> AppResult<()> {
    let result = sqlx::query(&format!(
        "DELETE FROM cron_schedules WHERE plugin_id = {}",
        ph(1)
    ))
    .bind(plugin_id)
    .execute(pool)
    .await?;

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

        let found = find_by_id(&pool, &s.document_id).await.unwrap().unwrap();
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

        toggle_schedule(&pool, &s.document_id, false).await.unwrap();
        let found = find_by_id(&pool, &s.document_id).await.unwrap().unwrap();
        assert!(!found.enabled);

        delete_schedule(&pool, &s.document_id).await.unwrap();
        assert!(find_by_id(&pool, &s.document_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn toggle_nonexistent_returns_not_found() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let result = toggle_schedule(&pool, "nonexistent", true).await;
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
        assert_eq!(list.len(), 2);

        // 第二次调用不会重复插入
        seed_defaults(&pool, &[]).await.unwrap();
        let list2 = list_schedules(&pool).await.unwrap();
        assert_eq!(list2.len(), 2);
    }

    #[tokio::test]
    async fn scheduler_dispatches_due_schedule() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let queue = std::sync::Arc::new(super::super::SqliteJobQueue::new(pool.clone()));

        // 手动插入一个 next_run_at 在过去的 schedule
        let now = Utc::now();
        let past = (now - chrono::Duration::hours(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO cron_schedules (document_id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)",
        )
        .bind("sched-1")
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

        let jobs = sqlx::query_as::<_, (String, String)>("SELECT document_id, job_type FROM jobs")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].1, "generate_sitemap");

        let row: (String,) =
            sqlx::query_as("SELECT next_run_at FROM cron_schedules WHERE document_id = 'sched-1'")
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

        let queue = std::sync::Arc::new(super::super::SqliteJobQueue::new(pool.clone()));

        let now = Utc::now();
        let future = (now + chrono::Duration::hours(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO cron_schedules (document_id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)",
        )
        .bind("sched-future")
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

    async fn insert_test_schedule(pool: &Pool, document_id: &str) -> i64 {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO cron_schedules (document_id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, 'Test', 'test_task', NULL, '0 */5 * * * *', 1, ?, NULL, ?, ?)",
        )
        .bind(document_id)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await
        .unwrap();

        let (id,): (i64,) = sqlx::query_as("SELECT id FROM cron_schedules WHERE document_id = ?")
            .bind(document_id)
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn execution_log_create_and_complete() {
        let pool = setup_log_tables().await;
        let sched_id = insert_test_schedule(&pool, "sched-1").await;

        let log_id = create_execution_log(&pool, sched_id, "generate_sitemap", "Sitemap")
            .await
            .unwrap();

        let logs = list_execution_logs(&pool, "sched-1", 10).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, CronExecStatus::Running);
        assert_eq!(logs[0].job_type, "generate_sitemap");
        assert!(logs[0].duration_ms.is_none());
        assert!(logs[0].finished_at.is_none());

        complete_execution_log(&pool, &log_id, 42).await.unwrap();

        let logs = list_execution_logs(&pool, "sched-1", 10).await.unwrap();
        assert_eq!(logs[0].status, CronExecStatus::Completed);
        assert_eq!(logs[0].duration_ms, Some(42));
        assert!(logs[0].finished_at.is_some());
    }

    #[tokio::test]
    async fn execution_log_fail_records_error() {
        let pool = setup_log_tables().await;
        let sched_id = insert_test_schedule(&pool, "sched-1").await;

        let log_id = create_execution_log(&pool, sched_id, "my_task", "Task")
            .await
            .unwrap();

        fail_execution_log(&pool, &log_id, 100, "something broke")
            .await
            .unwrap();

        let logs = list_execution_logs(&pool, "sched-1", 10).await.unwrap();
        assert_eq!(logs[0].status, CronExecStatus::Failed);
        assert_eq!(logs[0].duration_ms, Some(100));
        assert_eq!(logs[0].error, Some("something broke".into()));
    }

    #[tokio::test]
    async fn execution_log_list_by_schedule() {
        let pool = setup_log_tables().await;
        let sched_a = insert_test_schedule(&pool, "sched-a").await;
        let sched_b = insert_test_schedule(&pool, "sched-b").await;

        create_execution_log(&pool, sched_a, "task_a", "A")
            .await
            .unwrap();
        create_execution_log(&pool, sched_b, "task_b", "B")
            .await
            .unwrap();
        create_execution_log(&pool, sched_a, "task_a", "A2")
            .await
            .unwrap();

        let a = list_execution_logs(&pool, "sched-a", 10).await.unwrap();
        assert_eq!(a.len(), 2);

        let b = list_execution_logs(&pool, "sched-b", 10).await.unwrap();
        assert_eq!(b.len(), 1);
    }

    #[tokio::test]
    async fn execution_log_recent_ordering() {
        let pool = setup_log_tables().await;
        let s1 = insert_test_schedule(&pool, "s1").await;
        let s2 = insert_test_schedule(&pool, "s2").await;

        create_execution_log(&pool, s1, "task_1", "First")
            .await
            .unwrap();
        create_execution_log(&pool, s2, "task_2", "Second")
            .await
            .unwrap();

        let recent = recent_execution_logs(&pool, 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].label, "Second");
    }

    #[tokio::test]
    async fn execution_log_cleanup_removes_old() {
        let pool = setup_log_tables().await;
        let s1 = insert_test_schedule(&pool, "s1").await;

        create_execution_log(&pool, s1, "task_1", "Old")
            .await
            .unwrap();

        let count = cleanup_execution_logs(&pool, 0).await.unwrap();
        assert_eq!(count, 1);

        let logs = list_execution_logs(&pool, "s1", 10).await.unwrap();
        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn scheduler_dispatch_creates_execution_log() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let queue = std::sync::Arc::new(super::super::SqliteJobQueue::new(pool.clone()));
        let scheduler = CronScheduler::new(pool.clone(), queue, std::time::Duration::from_secs(60));

        let now = Utc::now();
        let past = (now - chrono::Duration::hours(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO cron_schedules (document_id, label, job_type, payload, cron_expr, enabled, next_run_at, plugin_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)",
        )
        .bind("sched-log-test")
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

        let logs = list_execution_logs(&pool, "sched-log-test", 10)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, CronExecStatus::Completed);
        assert!(logs[0].duration_ms.is_some());
        assert!(logs[0].finished_at.is_some());
    }
}

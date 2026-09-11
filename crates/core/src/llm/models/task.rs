//! `llm_tasks` model — generic LLM async-task rows (design: video now,
//! batch/image-async later). Strongly-typed lifecycle columns + kind-specific
//! request/response in `payload`/`result` JSON (`llm_logs` source+detail
//! pattern). Settlement bookkeeping (`pre_consumed`/`quota`) mirrors the
//! relay `PreCharge` semantics: completed settles, failed/expired refunds.

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::quota::Quota;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

define_enum!(
    LlmTaskKind {
        Video = "video",
    }
);

define_enum!(
    LlmTaskStatus {
        Queued = "queued",
        InProgress = "in_progress",
        Completed = "completed",
        Failed = "failed",
        Expired = "expired",
    }
);

impl LlmTaskStatus {
    /// Terminal states — no further polling, billing already settled.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            LlmTaskStatus::Completed | LlmTaskStatus::Failed | LlmTaskStatus::Expired
        )
    }
}

/// One async task row.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LlmTask {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub kind: LlmTaskKind,
    pub user_id: Option<SnowflakeId>,
    pub token_id: Option<SnowflakeId>,
    pub channel_id: Option<SnowflakeId>,
    pub key_index: Option<i32>,
    pub upstream_task_id: Option<String>,
    pub status: LlmTaskStatus,
    pub progress: i32,
    pub model_name: String,
    pub pre_consumed: Quota,
    pub quota: Quota,
    pub cost_quota: Quota,
    pub unlimited_quota: bool,
    pub payload: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Insert a freshly submitted task (pre-charge recorded for later
/// settle/refund).
#[allow(clippy::too_many_arguments)]
pub async fn create_task(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    kind: LlmTaskKind,
    user_id: SnowflakeId,
    token_id: SnowflakeId,
    channel_id: SnowflakeId,
    key_index: usize,
    upstream_task_id: &str,
    model_name: &str,
    pre_consumed: Quota,
    unlimited: bool,
    payload: &serde_json::Value,
    expires_at: Timestamp,
) -> AppResult<LlmTask> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "llm_tasks",
        [
            "id" => id,
            "kind" => kind.as_str(),
            "user_id" => user_id,
            "token_id" => token_id,
            "channel_id" => channel_id,
            "key_index" => key_index as i32,
            "upstream_task_id" => upstream_task_id,
            "status" => LlmTaskStatus::Queued.as_str(),
            "model_name" => model_name,
            "pre_consumed" => pre_consumed,
            "unlimited_quota" => unlimited,
            "payload" => payload,
            "expires_at" => expires_at,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("llm_task insert vanished")))
}

/// Find a task by id (tenant-scoped).
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<LlmTask>> {
    let result: Option<LlmTask> = raisfast_derive::crud_find!(
        pool,
        "llm_tasks",
        LlmTask,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

/// Terminal transition payload for [`finish_task`].
pub struct TaskOutcome {
    pub status: LlmTaskStatus,
    pub quota: Quota,
    pub cost_quota: Quota,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Terminal completion: settle `quota`/`cost_quota` and store `result`.
pub async fn finish_task(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    outcome: TaskOutcome,
) -> AppResult<()> {
    let now = now_utc();
    let progress = if outcome.status == LlmTaskStatus::Completed {
        100
    } else {
        0
    };
    let updated = raisfast_derive::crud_update!(
        pool,
        "llm_tasks",
        bind: [
            "status" => outcome.status.as_str(),
            "quota" => outcome.quota,
            "cost_quota" => outcome.cost_quota,
            "result" => outcome.result,
            "error_message" => outcome.error,
            "progress" => progress,
            "updated_at" => &now
        ],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&updated, "llm_task")
}

/// Non-terminal progress update (queued → in_progress, percent).
pub async fn update_progress(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    status: LlmTaskStatus,
    progress: i32,
) -> AppResult<()> {
    let now = now_utc();
    let updated = raisfast_derive::crud_update!(
        pool,
        "llm_tasks",
        bind: ["status" => status.as_str(), "progress" => progress, "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&updated, "llm_task")
}

/// Tasks whose expiry passed while still non-terminal — the refund sweep
/// marks them expired.
pub async fn find_expired(pool: &crate::db::Pool, now: Timestamp) -> AppResult<Vec<LlmTask>> {
    let rows: Vec<LlmTask> = raisfast_derive::crud_find_all!(
        pool,
        "llm_tasks",
        LlmTask,
        where: ("expires_at", LTE, now),
        order_by: "updated_at ASC"
    )?;
    Ok(rows
        .into_iter()
        .filter(|t| !t.status.is_terminal())
        .collect())
}

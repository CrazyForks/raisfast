//! 工作流数据模型与数据库查询

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::Pool;
use crate::db::dialect::ph;

/// 步骤类型
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "export-types", ts(rename_all = "snake_case"))]
pub enum StepType {
    /// 自动执行任务
    Task,
    /// 等待外部事件
    Await,
    /// 条件分支
    Branch,
    /// 并行执行
    Parallel,
    /// 延迟等待
    Delay,
}

/// 步骤定义
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDef {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "export-types", ts(rename = "type"))]
    pub step_type: StepType,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub config: serde_json::Value,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub next: serde_json::Value,
    #[serde(default)]
    pub timeout_ms: u64,
}

/// 工作流定义行
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowDefinition {
    pub id: i64,
    pub document_id: String,
    pub name: String,
    pub description: Option<String>,
    pub steps: String,
    pub initial_step: String,
    pub version: i64,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl WorkflowDefinition {
    /// 解析 steps JSON
    pub fn parse_steps(&self) -> anyhow::Result<Vec<StepDef>> {
        Ok(serde_json::from_str(&self.steps)?)
    }
}

/// 工作流实例行
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowInstance {
    pub id: i64,
    pub document_id: String,
    pub definition_id: i64,
    pub status: String,
    pub current_step: Option<String>,
    pub context: String,
    pub triggered_by: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

impl WorkflowInstance {
    /// 解析 context JSON
    pub fn parse_context(&self) -> serde_json::Value {
        serde_json::from_str(&self.context).unwrap_or(serde_json::json!({}))
    }
}

/// 步骤执行日志行
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StepLog {
    pub id: i64,
    pub document_id: String,
    pub instance_id: i64,
    pub step_id: String,
    pub step_name: String,
    pub status: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

/// 创建工作流定义
pub async fn create_definition(
    pool: &Pool,
    document_id: &str,
    name: &str,
    description: Option<&str>,
    steps: &str,
    initial_step: &str,
) -> anyhow::Result<WorkflowDefinition> {
    let now = crate::utils::tz::now_str();
    let sql = format!(
        "INSERT INTO workflow_definitions (document_id, name, description, steps, initial_step, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7)
    );
    sqlx::query(&sql)
        .bind(document_id)
        .bind(name)
        .bind(description)
        .bind(steps)
        .bind(initial_step)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
    get_definition(pool, document_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to fetch created workflow definition"))
}

/// 获取工作流定义
pub async fn get_definition(
    pool: &Pool,
    document_id: &str,
) -> anyhow::Result<Option<WorkflowDefinition>> {
    let sql = format!(
        "SELECT id, document_id, name, description, steps, initial_step, version, enabled, created_at, updated_at FROM workflow_definitions WHERE document_id = {}",
        ph(1)
    );
    let row = sqlx::query_as::<_, WorkflowDefinition>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 列出所有工作流定义
pub async fn list_definitions(pool: &Pool) -> anyhow::Result<Vec<WorkflowDefinition>> {
    let sql = "SELECT id, document_id, name, description, steps, initial_step, version, enabled, created_at, updated_at FROM workflow_definitions ORDER BY created_at DESC";
    let rows = sqlx::query_as::<_, WorkflowDefinition>(sql)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 删除工作流定义
pub async fn delete_definition(pool: &Pool, document_id: &str) -> anyhow::Result<()> {
    let sql = format!(
        "DELETE FROM workflow_definitions WHERE document_id = {}",
        ph(1)
    );
    sqlx::query(&sql).bind(document_id).execute(pool).await?;
    Ok(())
}

/// 创建工作流实例
pub async fn create_instance(
    pool: &Pool,
    document_id: &str,
    definition_id: i64,
    context: &serde_json::Value,
    triggered_by: Option<&str>,
) -> anyhow::Result<WorkflowInstance> {
    let now = crate::utils::tz::now_str();
    let ctx_str = serde_json::to_string(context)?;
    let sql = format!(
        "INSERT INTO workflow_instances (document_id, definition_id, status, context, triggered_by, started_at, updated_at) VALUES ({}, {}, 'running', {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6)
    );
    sqlx::query(&sql)
        .bind(document_id)
        .bind(definition_id)
        .bind(&ctx_str)
        .bind(triggered_by)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
    get_instance(pool, document_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to fetch created workflow instance"))
}

/// 获取工作流实例
pub async fn get_instance(
    pool: &Pool,
    document_id: &str,
) -> anyhow::Result<Option<WorkflowInstance>> {
    let sql = format!(
        "SELECT id, document_id, definition_id, status, current_step, context, triggered_by, started_at, completed_at, updated_at FROM workflow_instances WHERE document_id = {}",
        ph(1)
    );
    let row = sqlx::query_as::<_, WorkflowInstance>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 列出工作流实例
pub async fn list_instances(
    pool: &Pool,
    definition_id: Option<i64>,
    status: Option<&str>,
    page: i64,
    page_size: i64,
) -> anyhow::Result<(Vec<WorkflowInstance>, i64)> {
    let offset = (page - 1) * page_size;
    let sql = format!(
        "SELECT id, document_id, definition_id, status, current_step, context, triggered_by, started_at, completed_at, updated_at FROM workflow_instances WHERE ({} IS NULL OR definition_id = {}) AND ({} IS NULL OR status = {}) ORDER BY started_at DESC LIMIT {} OFFSET {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6)
    );
    let rows = sqlx::query_as::<_, WorkflowInstance>(&sql)
        .bind(definition_id)
        .bind(definition_id)
        .bind(status)
        .bind(status)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    let count_sql = format!(
        "SELECT COUNT(*) as count FROM workflow_instances WHERE ({} IS NULL OR definition_id = {}) AND ({} IS NULL OR status = {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4)
    );
    let (count,): (i64,) = sqlx::query_as(&count_sql)
        .bind(definition_id)
        .bind(definition_id)
        .bind(status)
        .bind(status)
        .fetch_one(pool)
        .await?;

    Ok((rows, count))
}

/// 更新实例状态和当前步骤
pub async fn update_instance_step(
    pool: &Pool,
    document_id: &str,
    status: &str,
    current_step: Option<&str>,
    context: &serde_json::Value,
) -> anyhow::Result<()> {
    let now = crate::utils::tz::now_str();
    let ctx_str = serde_json::to_string(context)?;
    let completed_at = if status == "completed" || status == "failed" || status == "cancelled" {
        Some(now.clone())
    } else {
        None
    };
    let sql = format!(
        "UPDATE workflow_instances SET status = {}, current_step = {}, context = {}, completed_at = COALESCE({}, completed_at), updated_at = {} WHERE document_id = {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6)
    );
    sqlx::query(&sql)
        .bind(status)
        .bind(current_step)
        .bind(&ctx_str)
        .bind(&completed_at)
        .bind(&now)
        .bind(document_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 创建步骤执行日志
pub async fn create_step_log(
    pool: &Pool,
    document_id: &str,
    instance_id: i64,
    step_id: &str,
    step_name: &str,
    input: Option<&serde_json::Value>,
) -> anyhow::Result<StepLog> {
    let now = crate::utils::tz::now_str();
    let input_str = input.map(|v| serde_json::to_string(v).unwrap_or_default());
    let sql = format!(
        "INSERT INTO workflow_step_logs (document_id, instance_id, step_id, step_name, status, input, started_at) VALUES ({}, {}, {}, {}, 'running', {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6)
    );
    sqlx::query(&sql)
        .bind(document_id)
        .bind(instance_id)
        .bind(step_id)
        .bind(step_name)
        .bind(&input_str)
        .bind(&now)
        .execute(pool)
        .await?;
    let fetch_sql = format!(
        "SELECT id, document_id, instance_id, step_id, step_name, status, input, output, error, started_at, completed_at FROM workflow_step_logs WHERE document_id = {}",
        ph(1)
    );
    sqlx::query_as::<_, StepLog>(&fetch_sql)
        .bind(document_id)
        .fetch_one(pool)
        .await
        .map_err(|e| anyhow::anyhow!("failed to fetch created step log: {e}"))
}

/// 完成步骤执行日志
pub async fn complete_step_log(
    pool: &Pool,
    document_id: &str,
    output: Option<&serde_json::Value>,
) -> anyhow::Result<()> {
    let now = crate::utils::tz::now_str();
    let output_str = output.map(|v| serde_json::to_string(v).unwrap_or_default());
    let sql = format!(
        "UPDATE workflow_step_logs SET status = 'completed', output = {}, completed_at = {} WHERE document_id = {}",
        ph(1),
        ph(2),
        ph(3)
    );
    sqlx::query(&sql)
        .bind(&output_str)
        .bind(&now)
        .bind(document_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 标记步骤执行失败
pub async fn fail_step_log(pool: &Pool, document_id: &str, error: &str) -> anyhow::Result<()> {
    let now = crate::utils::tz::now_str();
    let sql = format!(
        "UPDATE workflow_step_logs SET status = 'failed', error = {}, completed_at = {} WHERE document_id = {}",
        ph(1),
        ph(2),
        ph(3)
    );
    sqlx::query(&sql)
        .bind(error)
        .bind(&now)
        .bind(document_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 列出实例的步骤日志
pub async fn list_step_logs(pool: &Pool, instance_id: i64) -> anyhow::Result<Vec<StepLog>> {
    let sql = format!(
        "SELECT id, document_id, instance_id, step_id, step_name, status, input, output, error, started_at, completed_at FROM workflow_step_logs WHERE instance_id = {} ORDER BY started_at ASC",
        ph(1)
    );
    let rows = sqlx::query_as::<_, StepLog>(&sql)
        .bind(instance_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

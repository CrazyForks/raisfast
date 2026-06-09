//! Workflow data models and database queries

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::Pool;
use crate::db::{DbDriver, Driver};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    WorkflowInstanceStatus {
        Running = "running",
        Completed = "completed",
        Failed = "failed",
        Cancelled = "cancelled",
        Paused = "paused",
    }
);

define_enum!(
    WorkflowStepStatus {
        Running = "running",
        Completed = "completed",
        Failed = "failed",
    }
);

/// Step types
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "export-types", ts(rename_all = "snake_case"))]
pub enum StepType {
    /// Automatically executed task
    Task,
    /// Wait for external event
    Await,
    /// Conditional branch
    Branch,
    /// Parallel execution
    Parallel,
    /// Delay wait
    Delay,
}

/// Step definition
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

/// Workflow definition row
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowDefinition {
    pub id: SnowflakeId,
    pub name: String,
    pub description: Option<String>,
    pub steps: String,
    pub initial_step: String,
    pub version: i64,
    pub enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl WorkflowDefinition {
    /// Parse steps JSON
    pub fn parse_steps(&self) -> anyhow::Result<Vec<StepDef>> {
        Ok(serde_json::from_str(&self.steps)?)
    }
}

/// Workflow instance row
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowInstance {
    pub id: SnowflakeId,
    pub definition_id: SnowflakeId,
    pub status: WorkflowInstanceStatus,
    pub current_step: Option<String>,
    pub context: String,
    pub triggered_by: Option<SnowflakeId>,
    pub started_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

impl WorkflowInstance {
    /// Parse context JSON
    pub fn parse_context(&self) -> serde_json::Value {
        serde_json::from_str(&self.context).unwrap_or(serde_json::json!({}))
    }
}

/// Step execution log row
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StepLog {
    pub id: SnowflakeId,
    pub instance_id: SnowflakeId,
    pub step_id: String,
    pub step_name: String,
    pub status: WorkflowStepStatus,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub started_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}

/// Create workflow definition
pub async fn create_definition(
    pool: &Pool,
    id: SnowflakeId,
    name: &str,
    description: Option<&str>,
    steps: &str,
    initial_step: &str,
) -> anyhow::Result<WorkflowDefinition> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool, "workflow_definitions",
        ["id" => id, "name" => name, "description" => description, "steps" => steps, "initial_step" => initial_step, "created_at" => now, "updated_at" => now]
    )?;
    get_definition(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to fetch created workflow definition"))
}

/// Get workflow definition
pub async fn get_definition(
    pool: &Pool,
    id: SnowflakeId,
) -> anyhow::Result<Option<WorkflowDefinition>> {
    Ok(
        raisfast_derive::crud_find!(pool, "workflow_definitions", WorkflowDefinition, where: ("id", id))?,
    )
}

/// List all workflow definitions
pub async fn list_definitions(pool: &Pool) -> anyhow::Result<Vec<WorkflowDefinition>> {
    Ok(
        raisfast_derive::crud_list!(pool, "workflow_definitions", WorkflowDefinition, order_by: "created_at DESC")?,
    )
}

/// Delete workflow definition
pub async fn delete_definition(pool: &Pool, id: SnowflakeId) -> anyhow::Result<()> {
    raisfast_derive::crud_delete!(pool, "workflow_definitions", where: ("id", id))?;
    Ok(())
}

/// Create workflow instance
pub async fn create_instance(
    pool: &Pool,
    id: SnowflakeId,
    definition_id: SnowflakeId,
    context: &serde_json::Value,
    triggered_by: Option<i64>,
) -> anyhow::Result<WorkflowInstance> {
    let now = crate::utils::tz::now_utc();
    let ctx_str = serde_json::to_string(context)?;
    raisfast_derive::crud_insert!(
        pool, "workflow_instances",
        ["id" => id, "definition_id" => definition_id, "status" => WorkflowInstanceStatus::Running, "context" => ctx_str, "triggered_by" => triggered_by, "started_at" => now, "updated_at" => now]
    )?;
    get_instance(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to fetch created workflow instance"))
}

/// Get workflow instance
pub async fn get_instance(
    pool: &Pool,
    id: SnowflakeId,
) -> anyhow::Result<Option<WorkflowInstance>> {
    Ok(
        raisfast_derive::crud_find!(pool, "workflow_instances", WorkflowInstance, where: ("id", id))?,
    )
}

/// List workflow instances
pub async fn list_instances(
    pool: &Pool,
    definition_id: Option<i64>,
    status: Option<WorkflowInstanceStatus>,
    page: i64,
    page_size: i64,
) -> anyhow::Result<(Vec<WorkflowInstance>, i64)> {
    let offset = (page - 1) * page_size;
    let sql = format!(
        "SELECT id, definition_id, status, current_step, context, triggered_by, started_at, completed_at, updated_at FROM workflow_instances WHERE ({} IS NULL OR definition_id = {}) AND ({} IS NULL OR status = {}) ORDER BY started_at DESC LIMIT {} OFFSET {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6)
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
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4)
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

/// Update instance status and current step
pub async fn update_instance_step(
    pool: &Pool,
    id: SnowflakeId,
    status: WorkflowInstanceStatus,
    current_step: Option<&str>,
    context: &serde_json::Value,
) -> anyhow::Result<()> {
    let now = crate::utils::tz::now_utc();
    let ctx_str = serde_json::to_string(context)?;
    let completed_at = if status == WorkflowInstanceStatus::Completed
        || status == WorkflowInstanceStatus::Failed
        || status == WorkflowInstanceStatus::Cancelled
    {
        Some(now)
    } else {
        None
    };
    let sql = format!(
        "UPDATE workflow_instances SET status = {}, current_step = {}, context = {}, completed_at = COALESCE({}, completed_at), updated_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6)
    );
    sqlx::query(&sql)
        .bind(status)
        .bind(current_step)
        .bind(&ctx_str)
        .bind(completed_at)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Create step execution log
pub async fn create_step_log(
    pool: &Pool,
    id: SnowflakeId,
    instance_id: SnowflakeId,
    step_id: &str,
    step_name: &str,
    input: Option<&serde_json::Value>,
) -> anyhow::Result<StepLog> {
    let now = crate::utils::tz::now_utc();
    let input_str = input.map(|v| serde_json::to_string(v).unwrap_or_default());
    raisfast_derive::crud_insert!(
        pool, "workflow_step_logs",
        ["id" => id, "instance_id" => instance_id, "step_id" => step_id, "step_name" => step_name, "status" => WorkflowStepStatus::Running, "input" => input_str, "started_at" => now]
    )?;
    let result: StepLog =
        raisfast_derive::crud_find_one!(pool, "workflow_step_logs", StepLog, where: ("id", id))
            .map_err(|e: sqlx::Error| anyhow::anyhow!("failed to fetch created step log: {e}"))?;
    Ok(result)
}

/// Complete step execution log
pub async fn complete_step_log(
    pool: &Pool,
    id: SnowflakeId,
    output: Option<&serde_json::Value>,
) -> anyhow::Result<()> {
    let now = crate::utils::tz::now_utc();
    let output_str = output.map(|v| serde_json::to_string(v).unwrap_or_default());
    raisfast_derive::crud_update!(
        pool, "workflow_step_logs",
        bind: ["status" => WorkflowStepStatus::Completed, "output" => output_str, "completed_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

/// Mark step execution as failed
pub async fn fail_step_log(pool: &Pool, id: SnowflakeId, error: &str) -> anyhow::Result<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool, "workflow_step_logs",
        bind: ["status" => WorkflowStepStatus::Failed, "error" => error, "completed_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

/// List step logs for an instance
pub async fn list_step_logs(pool: &Pool, instance_id: SnowflakeId) -> anyhow::Result<Vec<StepLog>> {
    Ok(
        raisfast_derive::crud_find_all!(pool, "workflow_step_logs", StepLog, where: ("instance_id", instance_id), order_by: "started_at ASC")?,
    )
}

#[cfg(test)]
mod tests {
    use super::{WorkflowInstanceStatus, WorkflowStepStatus};
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    #[tokio::test]
    async fn create_and_get_definition() {
        let pool = setup_pool().await;
        let id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, id, "Test WF", Some("desc"), steps, "s1")
            .await
            .unwrap();
        assert_eq!(def.name, "Test WF");

        let got = super::get_definition(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.id, def.id);
    }

    #[tokio::test]
    async fn list_definitions() {
        let pool = setup_pool().await;
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        for i in 0..3 {
            let id = crate::utils::id::new_snowflake_id();
            super::create_definition(&pool, id, &format!("WF {i}"), None, steps, "s1")
                .await
                .unwrap();
        }
        let list = super::list_definitions(&pool).await.unwrap();
        assert_eq!(list.len(), 3);
    }

    #[tokio::test]
    async fn delete_definition() {
        let pool = setup_pool().await;
        let id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        super::create_definition(&pool, id, "To delete", None, steps, "s1")
            .await
            .unwrap();

        super::delete_definition(&pool, id).await.unwrap();
        assert!(super::get_definition(&pool, id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_and_get_instance() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        let inst_id = crate::utils::id::new_snowflake_id();
        let ctx = serde_json::json!({"key": "value"});
        let inst = super::create_instance(&pool, inst_id, def.id, &ctx, None)
            .await
            .unwrap();
        assert_eq!(inst.definition_id, def.id);
        assert_eq!(inst.status, WorkflowInstanceStatus::Running);

        let got = super::get_instance(&pool, inst_id).await.unwrap().unwrap();
        assert_eq!(got.id, inst.id);
    }

    #[tokio::test]
    async fn list_instances_test() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        for _ in 0..3 {
            let inst_id = crate::utils::id::new_snowflake_id();
            let ctx = serde_json::json!({});
            super::create_instance(&pool, inst_id, def.id, &ctx, None)
                .await
                .unwrap();
        }

        let (rows, count) = super::list_instances(&pool, Some(*def.id), None, 1, 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn update_instance_step() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        let inst_id = crate::utils::id::new_snowflake_id();
        let ctx = serde_json::json!({});
        super::create_instance(&pool, inst_id, def.id, &ctx, None)
            .await
            .unwrap();

        let new_ctx = serde_json::json!({"progress": 50});
        super::update_instance_step(
            &pool,
            inst_id,
            WorkflowInstanceStatus::Paused,
            Some("s1"),
            &new_ctx,
        )
        .await
        .unwrap();

        let inst = super::get_instance(&pool, inst_id).await.unwrap().unwrap();
        assert_eq!(inst.status, WorkflowInstanceStatus::Paused);
        assert_eq!(inst.current_step, Some("s1".to_string()));
    }

    #[tokio::test]
    async fn step_log_lifecycle() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        let inst_id = crate::utils::id::new_snowflake_id();
        let ctx = serde_json::json!({});
        let inst = super::create_instance(&pool, inst_id, def.id, &ctx, None)
            .await
            .unwrap();

        let log_id = crate::utils::id::new_snowflake_id();
        let input = serde_json::json!({"data": 1});
        let log = super::create_step_log(&pool, log_id, inst.id, "s1", "Step 1", Some(&input))
            .await
            .unwrap();
        assert_eq!(log.status, WorkflowStepStatus::Running);
        assert_eq!(log.step_id, "s1");

        let output = serde_json::json!({"result": "ok"});
        super::complete_step_log(&pool, log_id, Some(&output))
            .await
            .unwrap();

        let logs = super::list_step_logs(&pool, inst.id).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, WorkflowStepStatus::Completed);
        assert!(logs[0].completed_at.is_some());
    }

    #[tokio::test]
    async fn parse_steps_valid() {
        let pool = setup_pool().await;
        let id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, id, "WF", None, steps, "s1")
            .await
            .unwrap();

        let parsed = def.parse_steps().unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "s1");
        assert_eq!(parsed[0].step_type, super::StepType::Task);
    }

    #[tokio::test]
    async fn parse_steps_invalid_json() {
        let pool = setup_pool().await;
        let id = crate::utils::id::new_snowflake_id();
        let steps = "not valid json!!!";
        let def = super::create_definition(&pool, id, "WF", None, steps, "s1")
            .await
            .unwrap();

        assert!(def.parse_steps().is_err());
    }

    #[tokio::test]
    async fn fail_step_log_marks_failed() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        let inst_id = crate::utils::id::new_snowflake_id();
        let inst = super::create_instance(&pool, inst_id, def.id, &serde_json::json!({}), None)
            .await
            .unwrap();

        let log_id = crate::utils::id::new_snowflake_id();
        super::create_step_log(&pool, log_id, inst.id, "s1", "Step 1", None)
            .await
            .unwrap();

        super::fail_step_log(&pool, log_id, "something broke")
            .await
            .unwrap();

        let logs = super::list_step_logs(&pool, inst.id).await.unwrap();
        assert_eq!(logs[0].status, WorkflowStepStatus::Failed);
        assert_eq!(logs[0].error, Some("something broke".to_string()));
        assert!(logs[0].completed_at.is_some());
    }

    #[tokio::test]
    async fn list_instances_filter_by_status() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        let inst_id = crate::utils::id::new_snowflake_id();
        super::create_instance(&pool, inst_id, def.id, &serde_json::json!({}), None)
            .await
            .unwrap();

        super::update_instance_step(
            &pool,
            inst_id,
            WorkflowInstanceStatus::Completed,
            None,
            &serde_json::json!({}),
        )
        .await
        .unwrap();

        let (running, _) = super::list_instances(
            &pool,
            Some(*def.id),
            Some(WorkflowInstanceStatus::Running),
            1,
            10,
        )
        .await
        .unwrap();
        assert!(running.is_empty());

        let (completed, total) = super::list_instances(
            &pool,
            Some(*def.id),
            Some(WorkflowInstanceStatus::Completed),
            1,
            10,
        )
        .await
        .unwrap();
        assert_eq!(total, 1);
        assert_eq!(completed.len(), 1);
    }

    #[tokio::test]
    async fn list_instances_pagination() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        for _ in 0..5 {
            let inst_id = crate::utils::id::new_snowflake_id();
            super::create_instance(&pool, inst_id, def.id, &serde_json::json!({}), None)
                .await
                .unwrap();
        }

        let (page1, total) = super::list_instances(&pool, Some(*def.id), None, 1, 2)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 2);

        let (page2, _) = super::list_instances(&pool, Some(*def.id), None, 2, 2)
            .await
            .unwrap();
        assert_eq!(page2.len(), 2);
    }

    #[tokio::test]
    async fn update_instance_completed_sets_completed_at() {
        let pool = setup_pool().await;
        let def_id = crate::utils::id::new_snowflake_id();
        let steps = r#"[{"id":"s1","name":"Step 1","type":"task","config":{},"next":""}]"#;
        let def = super::create_definition(&pool, def_id, "WF", None, steps, "s1")
            .await
            .unwrap();

        let inst_id = crate::utils::id::new_snowflake_id();
        super::create_instance(&pool, inst_id, def.id, &serde_json::json!({}), None)
            .await
            .unwrap();

        super::update_instance_step(
            &pool,
            inst_id,
            WorkflowInstanceStatus::Completed,
            None,
            &serde_json::json!({"done": true}),
        )
        .await
        .unwrap();

        let inst = super::get_instance(&pool, inst_id).await.unwrap().unwrap();
        assert_eq!(inst.status, WorkflowInstanceStatus::Completed);
        assert!(inst.completed_at.is_some());
        let ctx = inst.parse_context();
        assert_eq!(ctx["done"], true);
    }

    #[tokio::test]
    async fn get_definition_not_found() {
        let pool = setup_pool().await;
        let result = super::get_definition(&pool, SnowflakeId(9999999))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_instance_not_found() {
        let pool = setup_pool().await;
        let result = super::get_instance(&pool, SnowflakeId(9999999))
            .await
            .unwrap();
        assert!(result.is_none());
    }
}

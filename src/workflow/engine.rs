//! 工作流引擎 — 状态机核心

use super::model::{
    StepDef, StepType, WorkflowDefinition, WorkflowInstance, WorkflowInstanceStatus,
    WorkflowStepStatus, complete_step_log, create_definition, create_instance, create_step_log,
    fail_step_log, get_definition, get_instance, list_definitions, list_step_logs,
    update_instance_step,
};
use super::validate::{resolve_next_step, validate_steps};
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use serde_json::json;

const PARALLEL_META_KEY: &str = "_parallel";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ParallelMeta {
    /// Parallel step ID that spawned the branches
    parent: String,
    /// Branch step IDs still pending completion
    pending: Vec<String>,
    /// Step ID to advance to after all branches complete (None = end workflow)
    join_next: Option<String>,
}

/// 工作流服务
pub struct WorkflowService {
    pool: Pool,
}

impl WorkflowService {
    /// 创建新的工作流服务
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// 创建工作流定义
    pub async fn create_workflow(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        steps: &[StepDef],
    ) -> AppResult<WorkflowDefinition> {
        let initial_step = steps
            .first()
            .ok_or_else(|| AppError::BadRequest("workflow must have at least one step".into()))?
            .id
            .clone();

        validate_steps(steps)?;

        let steps_json = serde_json::to_string(steps)
            .map_err(|e| AppError::BadRequest(format!("invalid steps JSON: {e}")))?;

        create_definition(
            &self.pool,
            id,
            name,
            description,
            &steps_json,
            &initial_step,
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }

    /// 获取工作流定义
    pub async fn get_workflow(&self, id: &str) -> AppResult<WorkflowDefinition> {
        get_definition(&self.pool, id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
            .ok_or_else(|| AppError::not_found("workflow"))
    }

    /// 列出所有工作流定义
    pub async fn list_workflows(&self) -> AppResult<Vec<WorkflowDefinition>> {
        list_definitions(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }

    async fn get_definition_by_pk(&self, id: i64) -> AppResult<WorkflowDefinition> {
        let sql = format!(
            "SELECT id, document_id, name, description, steps, initial_step, version, enabled, created_at, updated_at FROM workflow_definitions WHERE id = {}",
            crate::db::dialect::ph(1)
        );
        sqlx::query_as::<_, WorkflowDefinition>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
            .ok_or_else(|| AppError::not_found("workflow definition"))
    }

    /// 删除工作流定义
    pub async fn delete_workflow(&self, id: &str) -> AppResult<()> {
        super::model::delete_definition(&self.pool, id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }

    /// 启动工作流实例
    pub async fn start_workflow(
        &self,
        definition_id: &str,
        context: &serde_json::Value,
        triggered_by: Option<i64>,
    ) -> AppResult<WorkflowInstance> {
        let def = self.get_workflow(definition_id).await?;

        if !def.enabled {
            return Err(AppError::BadRequest("workflow is disabled".into()));
        }

        let id = uuid::Uuid::now_v7().to_string();
        let instance = create_instance(&self.pool, &id, def.id, context, triggered_by)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        update_instance_step(
            &self.pool,
            &id,
            WorkflowInstanceStatus::Running,
            Some(&def.initial_step),
            context,
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        let steps = def
            .parse_steps()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        let initial = steps
            .iter()
            .find(|s| s.id == def.initial_step)
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("initial step not found")))?;

        let log_id = uuid::Uuid::now_v7().to_string();
        create_step_log(
            &self.pool,
            &log_id,
            instance.id,
            &initial.id,
            &initial.name,
            Some(context),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        Ok(WorkflowInstance {
            current_step: Some(def.initial_step),
            ..instance
        })
    }

    /// 执行工作流的当前步骤并推进状态
    ///
    /// 核心状态转换逻辑：根据步骤类型执行操作，
    /// 然后根据 next 配置确定下一步。
    ///
    /// Parallel 步骤会启动所有分支，每个分支通过后续的 execute_step 调用依次完成。
    /// 所有分支完成后自动推进到 join_next 步骤。
    pub async fn execute_step(
        &self,
        instance_id: &str,
        step_output: &serde_json::Value,
    ) -> AppResult<WorkflowInstance> {
        let instance = self
            .get_instance(instance_id)
            .await?
            .ok_or_else(|| AppError::not_found("workflow instance"))?;

        if instance.status != WorkflowInstanceStatus::Running {
            return Err(AppError::BadRequest(
                "workflow instance is not running".into(),
            ));
        }

        let current_step_id = instance
            .current_step
            .as_deref()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("no current step")))?;

        let def = self.get_definition_by_pk(instance.definition_id).await?;
        let steps = def
            .parse_steps()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        let current_step = steps
            .iter()
            .find(|s| s.id == current_step_id)
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("step not found: {current_step_id}"))
            })?;

        let mut context = instance.parse_context();
        merge_output_into_context(&mut context, step_output);

        match current_step.step_type {
            StepType::Parallel => {
                self.execute_parallel_step(&instance, current_step, &steps, &mut context)
                    .await
            }
            _ => {
                let parallel_meta = extract_parallel_meta(&context);
                if let Some(meta) = &parallel_meta
                    && meta.pending.contains(&current_step.id)
                {
                    return self
                        .execute_parallel_branch(
                            &instance,
                            current_step,
                            &steps,
                            &mut context,
                            meta,
                        )
                        .await;
                }
                self.execute_normal_step(&instance, current_step, &steps, step_output, &mut context)
                    .await
            }
        }
    }

    /// 执行普通步骤（Task/Await/Delay/Branch）
    async fn execute_normal_step(
        &self,
        instance: &WorkflowInstance,
        current_step: &StepDef,
        steps: &[StepDef],
        step_output: &serde_json::Value,
        context: &mut serde_json::Value,
    ) -> AppResult<WorkflowInstance> {
        let current_step_id = &current_step.id;

        let active_logs: Vec<_> = list_step_logs(&self.pool, instance.id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
            .into_iter()
            .filter(|l| l.step_id == *current_step_id && (l.status == WorkflowStepStatus::Running))
            .collect();

        if let Some(log) = active_logs.first() {
            complete_step_log(&self.pool, &log.document_id, Some(step_output))
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        }

        let next_step_id = resolve_next_step(current_step, context);

        match next_step_id {
            Some(next_id) => {
                let next_step = steps.iter().find(|s| s.id == next_id);
                match next_step {
                    Some(ns) => {
                        update_instance_step(
                            &self.pool,
                            &instance.document_id,
                            WorkflowInstanceStatus::Running,
                            Some(&ns.id),
                            context,
                        )
                        .await
                        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                        let log_id = uuid::Uuid::now_v7().to_string();
                        create_step_log(
                            &self.pool,
                            &log_id,
                            instance.id,
                            &ns.id,
                            &ns.name,
                            Some(context),
                        )
                        .await
                        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                        self.get_instance(&instance.document_id)
                            .await?
                            .ok_or_else(|| AppError::not_found("workflow instance"))
                    }
                    None => Err(AppError::Internal(anyhow::anyhow!(
                        "next step not found: {next_id}"
                    ))),
                }
            }
            None => {
                update_instance_step(
                    &self.pool,
                    &instance.document_id,
                    WorkflowInstanceStatus::Completed,
                    None,
                    context,
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                self.get_instance(&instance.document_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("workflow instance"))
            }
        }
    }

    /// Parallel 步骤被触发时：启动所有并行分支
    async fn execute_parallel_step(
        &self,
        instance: &WorkflowInstance,
        parallel_step: &StepDef,
        steps: &[StepDef],
        context: &mut serde_json::Value,
    ) -> AppResult<WorkflowInstance> {
        let branches = parallel_step.next.as_array().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!(
                "parallel step '{}' must have array next",
                parallel_step.id
            ))
        })?;

        let branch_ids: Vec<String> = branches
            .iter()
            .filter_map(|b| b.as_str().map(String::from))
            .collect();

        if branch_ids.is_empty() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "parallel step '{}' has no branches",
                parallel_step.id
            )));
        }

        let join_next = parallel_step
            .config
            .get("join_next")
            .and_then(|v| v.as_str())
            .map(String::from);

        if let Some(ref jn) = join_next
            && !steps.iter().any(|s| s.id == *jn)
        {
            return Err(AppError::Internal(anyhow::anyhow!(
                "parallel join_next '{}' not found in steps",
                jn
            )));
        }

        let active_logs: Vec<_> = list_step_logs(&self.pool, instance.id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
            .into_iter()
            .filter(|l| l.step_id == parallel_step.id && (l.status == WorkflowStepStatus::Running))
            .collect();
        if let Some(log) = active_logs.first() {
            complete_step_log(&self.pool, &log.document_id, None)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        }

        let meta = ParallelMeta {
            parent: parallel_step.id.clone(),
            pending: branch_ids.clone(),
            join_next,
        };
        context[PARALLEL_META_KEY] =
            serde_json::to_value(&meta).map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        let first_branch = steps
            .iter()
            .find(|s| s.id == branch_ids[0])
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "parallel branch '{}' not found",
                    branch_ids[0]
                ))
            })?;

        for branch_id in &branch_ids {
            let branch_step = steps.iter().find(|s| s.id == *branch_id).ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("parallel branch '{}' not found", branch_id))
            })?;
            let log_id = uuid::Uuid::now_v7().to_string();
            create_step_log(
                &self.pool,
                &log_id,
                instance.id,
                &branch_step.id,
                &branch_step.name,
                Some(context),
            )
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        }

        update_instance_step(
            &self.pool,
            &instance.document_id,
            WorkflowInstanceStatus::Running,
            Some(&first_branch.id),
            context,
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        self.get_instance(&instance.document_id)
            .await?
            .ok_or_else(|| AppError::not_found("workflow instance"))
    }

    /// 完成一个并行分支：检查是否所有分支都已完成
    async fn execute_parallel_branch(
        &self,
        instance: &WorkflowInstance,
        branch_step: &StepDef,
        steps: &[StepDef],
        context: &mut serde_json::Value,
        meta: &ParallelMeta,
    ) -> AppResult<WorkflowInstance> {
        let active_logs: Vec<_> = list_step_logs(&self.pool, instance.id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
            .into_iter()
            .filter(|l| l.step_id == branch_step.id && (l.status == WorkflowStepStatus::Running))
            .collect();
        if let Some(log) = active_logs.first() {
            complete_step_log(&self.pool, &log.document_id, None)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        }

        let remaining: Vec<String> = meta
            .pending
            .iter()
            .filter(|id| **id != branch_step.id)
            .cloned()
            .collect();

        if !remaining.is_empty() {
            let next_branch = steps.iter().find(|s| s.id == remaining[0]).ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "parallel branch '{}' not found",
                    remaining[0]
                ))
            })?;

            let updated_meta = ParallelMeta {
                pending: remaining,
                ..meta.clone()
            };
            context[PARALLEL_META_KEY] = serde_json::to_value(&updated_meta)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

            update_instance_step(
                &self.pool,
                &instance.document_id,
                WorkflowInstanceStatus::Running,
                Some(&next_branch.id),
                context,
            )
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

            return self
                .get_instance(&instance.document_id)
                .await?
                .ok_or_else(|| AppError::not_found("workflow instance"));
        }

        context
            .as_object_mut()
            .map(|obj| obj.remove(PARALLEL_META_KEY));

        match &meta.join_next {
            Some(join_id) => {
                let join_step = steps.iter().find(|s| s.id == *join_id).ok_or_else(|| {
                    AppError::Internal(anyhow::anyhow!(
                        "parallel join_next '{}' not found",
                        join_id
                    ))
                })?;

                update_instance_step(
                    &self.pool,
                    &instance.document_id,
                    WorkflowInstanceStatus::Running,
                    Some(&join_step.id),
                    context,
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                let log_id = uuid::Uuid::now_v7().to_string();
                create_step_log(
                    &self.pool,
                    &log_id,
                    instance.id,
                    &join_step.id,
                    &join_step.name,
                    Some(context),
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                self.get_instance(&instance.document_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("workflow instance"))
            }
            None => {
                update_instance_step(
                    &self.pool,
                    &instance.document_id,
                    WorkflowInstanceStatus::Completed,
                    None,
                    context,
                )
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                self.get_instance(&instance.document_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("workflow instance"))
            }
        }
    }

    /// 标记步骤失败
    pub async fn fail_step(&self, instance_id: &str, error: &str) -> AppResult<()> {
        let instance = self
            .get_instance(instance_id)
            .await?
            .ok_or_else(|| AppError::not_found("workflow instance"))?;

        if let Some(ref step_id) = instance.current_step {
            let active_logs: Vec<_> = list_step_logs(&self.pool, instance.id)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
                .into_iter()
                .filter(|l| l.step_id == *step_id && (l.status == WorkflowStepStatus::Running))
                .collect();

            if let Some(log) = active_logs.first() {
                fail_step_log(&self.pool, &log.document_id, error)
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
            }
        }

        update_instance_step(
            &self.pool,
            instance_id,
            WorkflowInstanceStatus::Failed,
            None,
            &json!({}),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        Ok(())
    }

    /// 取消工作流实例
    pub async fn cancel_instance(&self, instance_id: &str) -> AppResult<()> {
        let instance = self
            .get_instance(instance_id)
            .await?
            .ok_or_else(|| AppError::not_found("workflow instance"))?;

        if instance.status != WorkflowInstanceStatus::Running {
            return Err(AppError::BadRequest(
                "only running instances can be cancelled".into(),
            ));
        }

        update_instance_step(
            &self.pool,
            instance_id,
            WorkflowInstanceStatus::Cancelled,
            None,
            &json!({}),
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        Ok(())
    }

    /// 获取工作流实例
    pub async fn get_instance(&self, id: &str) -> AppResult<Option<WorkflowInstance>> {
        get_instance(&self.pool, id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }

    /// 列出工作流实例
    pub async fn list_instances(
        &self,
        definition_id: Option<&str>,
        status: Option<WorkflowInstanceStatus>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WorkflowInstance>, i64)> {
        let def_id: Option<i64> = match definition_id {
            Some(did) => Some(self.get_workflow(did).await?.id),
            None => None,
        };
        super::model::list_instances(&self.pool, def_id, status, page, page_size)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }

    /// 获取步骤日志
    pub async fn get_step_logs(&self, instance_id: &str) -> AppResult<Vec<super::model::StepLog>> {
        let instance = self
            .get_instance(instance_id)
            .await?
            .ok_or_else(|| AppError::not_found("workflow instance"))?;
        list_step_logs(&self.pool, instance.id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }
}

fn extract_parallel_meta(context: &serde_json::Value) -> Option<ParallelMeta> {
    context
        .get(PARALLEL_META_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// 将步骤输出合并到工作流 context
pub(crate) fn merge_output_into_context(
    context: &mut serde_json::Value,
    output: &serde_json::Value,
) {
    if let (Some(ctx_obj), Some(out_obj)) = (context.as_object_mut(), output.as_object()) {
        for (k, v) in out_obj {
            ctx_obj.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::model::{StepDef, StepType};

    async fn setup() -> WorkflowService {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        WorkflowService::new(pool)
    }

    fn task_step(id: &str, name: &str, next: &str) -> StepDef {
        StepDef {
            id: id.to_string(),
            name: name.to_string(),
            step_type: StepType::Task,
            config: json!({}),
            next: json!(next),
            timeout_ms: 0,
        }
    }

    fn await_step(id: &str, name: &str, next: &str) -> StepDef {
        StepDef {
            id: id.to_string(),
            name: name.to_string(),
            step_type: StepType::Await,
            config: json!({}),
            next: json!(next),
            timeout_ms: 0,
        }
    }

    fn branch_step(id: &str, name: &str, next: serde_json::Value) -> StepDef {
        StepDef {
            id: id.to_string(),
            name: name.to_string(),
            step_type: StepType::Branch,
            config: json!({}),
            next,
            timeout_ms: 0,
        }
    }

    fn parallel_step(id: &str, name: &str, branches: &[&str], join_next: Option<&str>) -> StepDef {
        StepDef {
            id: id.to_string(),
            name: name.to_string(),
            step_type: StepType::Parallel,
            config: match join_next {
                Some(jn) => json!({"join_next": jn}),
                None => json!({}),
            },
            next: json!(branches),
            timeout_ms: 0,
        }
    }

    #[tokio::test]
    async fn create_workflow_success() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "Step 1", "")];
        let def = svc
            .create_workflow("wf-1", "Test WF", Some("desc"), &steps)
            .await
            .unwrap();
        assert_eq!(def.name, "Test WF");
        assert_eq!(def.initial_step, "s1");
        assert_eq!(def.document_id, "wf-1");
        assert!(def.enabled);
    }

    #[tokio::test]
    async fn create_workflow_empty_steps_rejected() {
        let svc = setup().await;
        let result = svc.create_workflow("wf-empty", "Empty", None, &[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_workflow_found() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-get", "Get", None, &steps)
            .await
            .unwrap();
        let def = svc.get_workflow("wf-get").await.unwrap();
        assert_eq!(def.name, "Get");
    }

    #[tokio::test]
    async fn get_workflow_not_found() {
        let svc = setup().await;
        let result = svc.get_workflow("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_workflows() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-a", "A", None, &steps)
            .await
            .unwrap();
        svc.create_workflow("wf-b", "B", None, &steps)
            .await
            .unwrap();
        let list = svc.list_workflows().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn delete_workflow() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-del", "Del", None, &steps)
            .await
            .unwrap();
        svc.delete_workflow("wf-del").await.unwrap();
        assert!(svc.get_workflow("wf-del").await.is_err());
    }

    #[tokio::test]
    async fn start_workflow_success() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "Step 1", "")];
        svc.create_workflow("wf-start", "Start", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-start", &json!({"key": "val"}), None)
            .await
            .unwrap();
        assert_eq!(inst.status, WorkflowInstanceStatus::Running);
        assert_eq!(inst.current_step, Some("s1".to_string()));
    }

    #[tokio::test]
    async fn start_workflow_not_found() {
        let svc = setup().await;
        let result = svc.start_workflow("nope", &json!({}), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn start_workflow_disabled() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        let def = svc
            .create_workflow("wf-dis", "Dis", None, &steps)
            .await
            .unwrap();
        sqlx::query("UPDATE workflow_definitions SET enabled = 0 WHERE id = ?")
            .bind(def.id)
            .execute(&svc.pool)
            .await
            .unwrap();

        let result = svc.start_workflow("wf-dis", &json!({}), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_step_completes_single_task() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "Only", "")];
        svc.create_workflow("wf-exec1", "Exec", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-exec1", &json!({}), None)
            .await
            .unwrap();
        let inst_id = &inst.document_id;

        let result = svc
            .execute_step(inst_id, &json!({"done": true}))
            .await
            .unwrap();
        assert_eq!(result.status, WorkflowInstanceStatus::Completed);
        assert!(result.current_step.is_none());
    }

    #[tokio::test]
    async fn execute_step_advances_to_next() {
        let svc = setup().await;
        let steps = vec![
            task_step("s1", "First", "s2"),
            task_step("s2", "Second", ""),
        ];
        svc.create_workflow("wf-exec2", "Chain", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-exec2", &json!({}), None)
            .await
            .unwrap();
        let inst_id = inst.document_id.clone();

        let result = svc.execute_step(&inst_id, &json!({})).await.unwrap();
        assert_eq!(result.current_step, Some("s2".to_string()));
        assert_eq!(result.status, WorkflowInstanceStatus::Running);
    }

    #[tokio::test]
    async fn execute_step_not_running_rejected() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-nr", "NR", None, &steps)
            .await
            .unwrap();

        let inst = svc.start_workflow("wf-nr", &json!({}), None).await.unwrap();
        svc.execute_step(&inst.document_id, &json!({}))
            .await
            .unwrap();

        let result = svc.execute_step(&inst.document_id, &json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_step_instance_not_found() {
        let svc = setup().await;
        let result = svc.execute_step("nope", &json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_branch_condition_match() {
        let svc = setup().await;
        let steps = vec![
            branch_step(
                "s1",
                "Decide",
                json!([
                    {"condition": {"ok": true}, "step": "s2"},
                    {"step": "s3"}
                ]),
            ),
            task_step("s2", "Yes", ""),
            task_step("s3", "No", ""),
        ];
        svc.create_workflow("wf-br", "Branch", None, &steps)
            .await
            .unwrap();

        let inst = svc.start_workflow("wf-br", &json!({}), None).await.unwrap();
        let result = svc
            .execute_step(&inst.document_id, &json!({"ok": true}))
            .await
            .unwrap();
        assert_eq!(result.current_step, Some("s2".to_string()));
    }

    #[tokio::test]
    async fn execute_branch_fallthrough() {
        let svc = setup().await;
        let steps = vec![
            branch_step(
                "s1",
                "Decide",
                json!([
                    {"condition": {"ok": true}, "step": "s2"},
                    {"step": "s3"}
                ]),
            ),
            task_step("s2", "Yes", ""),
            task_step("s3", "No", ""),
        ];
        svc.create_workflow("wf-br2", "Branch2", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-br2", &json!({}), None)
            .await
            .unwrap();
        let result = svc
            .execute_step(&inst.document_id, &json!({"ok": false}))
            .await
            .unwrap();
        assert_eq!(result.current_step, Some("s3".to_string()));
    }

    #[tokio::test]
    async fn execute_parallel_with_join() {
        let svc = setup().await;
        let steps = vec![
            parallel_step("s1", "Par", &["s2", "s3"], Some("s4")),
            task_step("s2", "A", ""),
            task_step("s3", "B", ""),
            task_step("s4", "After", ""),
        ];
        svc.create_workflow("wf-par", "Par", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-par", &json!({}), None)
            .await
            .unwrap();
        let iid = inst.document_id.clone();

        let r = svc.execute_step(&iid, &json!({})).await.unwrap();
        assert_eq!(r.current_step, Some("s2".to_string()));

        let r = svc.execute_step(&iid, &json!({"a": 1})).await.unwrap();
        assert_eq!(r.current_step, Some("s3".to_string()));

        let r = svc.execute_step(&iid, &json!({"b": 2})).await.unwrap();
        assert_eq!(r.current_step, Some("s4".to_string()));

        let r = svc.execute_step(&iid, &json!({})).await.unwrap();
        assert_eq!(r.status, WorkflowInstanceStatus::Completed);
    }

    #[tokio::test]
    async fn execute_parallel_no_join_completes() {
        let svc = setup().await;
        let steps = vec![
            parallel_step("s1", "Par", &["s2", "s3"], None),
            task_step("s2", "A", ""),
            task_step("s3", "B", ""),
        ];
        svc.create_workflow("wf-par2", "Par2", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-par2", &json!({}), None)
            .await
            .unwrap();
        let iid = inst.document_id.clone();

        svc.execute_step(&iid, &json!({})).await.unwrap();
        svc.execute_step(&iid, &json!({})).await.unwrap();
        let r = svc.execute_step(&iid, &json!({})).await.unwrap();
        assert_eq!(r.status, WorkflowInstanceStatus::Completed);
    }

    #[tokio::test]
    async fn fail_step_marks_failed() {
        let svc = setup().await;
        let steps = vec![await_step("s1", "Wait", "")];
        svc.create_workflow("wf-fail", "Fail", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-fail", &json!({}), None)
            .await
            .unwrap();
        svc.fail_step(&inst.document_id, "timeout").await.unwrap();

        let inst = svc.get_instance(&inst.document_id).await.unwrap().unwrap();
        assert_eq!(inst.status, WorkflowInstanceStatus::Failed);
    }

    #[tokio::test]
    async fn fail_step_not_found() {
        let svc = setup().await;
        let result = svc.fail_step("nope", "err").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cancel_instance() {
        let svc = setup().await;
        let steps = vec![await_step("s1", "Wait", "")];
        svc.create_workflow("wf-can", "Cancel", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-can", &json!({}), None)
            .await
            .unwrap();
        svc.cancel_instance(&inst.document_id).await.unwrap();

        let inst = svc.get_instance(&inst.document_id).await.unwrap().unwrap();
        assert_eq!(inst.status, WorkflowInstanceStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_not_running_rejected() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-can2", "Cancel2", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-can2", &json!({}), None)
            .await
            .unwrap();
        svc.execute_step(&inst.document_id, &json!({}))
            .await
            .unwrap();

        let result = svc.cancel_instance(&inst.document_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_step_logs_returns_entries() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-logs", "Logs", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-logs", &json!({}), None)
            .await
            .unwrap();
        svc.execute_step(&inst.document_id, &json!({"x": 1}))
            .await
            .unwrap();

        let logs = svc.get_step_logs(&inst.document_id).await.unwrap();
        assert!(!logs.is_empty());
        assert_eq!(logs[0].step_id, "s1");
        assert_eq!(logs[0].status, WorkflowStepStatus::Completed);
    }

    #[tokio::test]
    async fn get_step_logs_instance_not_found() {
        let svc = setup().await;
        let result = svc.get_step_logs("nope").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_instances_by_definition() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-li", "List", None, &steps)
            .await
            .unwrap();

        svc.start_workflow("wf-li", &json!({}), None).await.unwrap();
        svc.start_workflow("wf-li", &json!({}), None).await.unwrap();

        let (items, total) = svc
            .list_instances(Some("wf-li"), None, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn list_instances_by_status() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-ls", "Status", None, &steps)
            .await
            .unwrap();

        let inst = svc.start_workflow("wf-ls", &json!({}), None).await.unwrap();
        svc.execute_step(&inst.document_id, &json!({}))
            .await
            .unwrap();

        let (running, _) = svc
            .list_instances(None, Some(WorkflowInstanceStatus::Running), 1, 10)
            .await
            .unwrap();
        assert!(running.is_empty());

        let (completed, total) = svc
            .list_instances(None, Some(WorkflowInstanceStatus::Completed), 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(completed.len(), 1);
    }

    #[tokio::test]
    async fn list_instances_pagination() {
        let svc = setup().await;
        let steps = vec![task_step("s1", "S1", "")];
        svc.create_workflow("wf-page", "Page", None, &steps)
            .await
            .unwrap();

        for _ in 0..5 {
            svc.start_workflow("wf-page", &json!({}), None)
                .await
                .unwrap();
        }

        let (page1, total) = svc
            .list_instances(Some("wf-page"), None, 1, 2)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 2);

        let (page2, _) = svc
            .list_instances(Some("wf-page"), None, 2, 2)
            .await
            .unwrap();
        assert_eq!(page2.len(), 2);
    }

    #[tokio::test]
    async fn context_accumulates_across_steps() {
        let svc = setup().await;
        let steps = vec![
            task_step("s1", "First", "s2"),
            task_step("s2", "Second", ""),
        ];
        svc.create_workflow("wf-ctx", "Ctx", None, &steps)
            .await
            .unwrap();

        let inst = svc
            .start_workflow("wf-ctx", &json!({"init": 1}), None)
            .await
            .unwrap();
        let iid = inst.document_id.clone();

        svc.execute_step(&iid, &json!({"step1": "done"}))
            .await
            .unwrap();
        svc.execute_step(&iid, &json!({"step2": "done"}))
            .await
            .unwrap();

        let inst = svc.get_instance(&iid).await.unwrap().unwrap();
        let ctx = inst.parse_context();
        assert_eq!(ctx["init"], 1);
        assert_eq!(ctx["step1"], "done");
        assert_eq!(ctx["step2"], "done");
    }

    #[test]
    fn merge_output_adds_keys() {
        let mut ctx = json!({"name": "test"});
        let output = json!({"result": "ok", "count": 5});
        merge_output_into_context(&mut ctx, &output);
        assert_eq!(ctx["result"], "ok");
        assert_eq!(ctx["count"], 5);
        assert_eq!(ctx["name"], "test");
    }

    #[test]
    fn merge_output_preserves_existing_keys() {
        let mut ctx = json!({"a": 1, "b": 2});
        merge_output_into_context(&mut ctx, &json!({"b": 99, "c": 3}));
        assert_eq!(ctx["a"], 1);
        assert_eq!(ctx["b"], 99);
        assert_eq!(ctx["c"], 3);
    }

    #[test]
    fn merge_output_non_object_context_noop() {
        let mut ctx = json!("string");
        merge_output_into_context(&mut ctx, &json!({"a": 1}));
        assert_eq!(ctx, json!("string"));
    }
}

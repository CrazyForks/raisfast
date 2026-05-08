//! 工作流引擎服务层
//!
//! 提供工作流定义管理、实例创建、步骤执行和状态转换。
//!
//! ## 工作流执行模型
//!
//! 每个工作流由一组有序步骤组成，支持：
//! - **Task**: 自动执行（调用 plugin hook / 内置服务）
//! - **Await**: 等待外部事件（如审批操作）
//! - **Branch**: 条件分支（根据 context 选择下一步）
//! - **Parallel**: 并行执行（所有分支完成后继续）
//! - **Delay**: 延迟等待

use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::models::workflow::{
    StepDef, StepType, WorkflowDefinition, WorkflowInstance, complete_step_log, create_definition,
    create_instance, create_step_log, fail_step_log, get_definition, get_instance,
    list_definitions, list_step_logs, update_instance_step,
};
use serde_json::json;

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
        crate::models::workflow::delete_definition(&self.pool, id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }

    /// 启动工作流实例
    pub async fn start_workflow(
        &self,
        definition_id: &str,
        context: &serde_json::Value,
        triggered_by: Option<&str>,
    ) -> AppResult<WorkflowInstance> {
        let def = self.get_workflow(definition_id).await?;

        if !def.enabled {
            return Err(AppError::BadRequest("workflow is disabled".into()));
        }

        let id = uuid::Uuid::now_v7().to_string();
        let instance = create_instance(&self.pool, &id, def.id, context, triggered_by)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        update_instance_step(&self.pool, &id, "running", Some(&def.initial_step), context)
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
    pub async fn execute_step(
        &self,
        instance_id: &str,
        step_output: &serde_json::Value,
    ) -> AppResult<WorkflowInstance> {
        let instance = self
            .get_instance(instance_id)
            .await?
            .ok_or_else(|| AppError::not_found("workflow instance"))?;

        if instance.status != "running" {
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

        let active_logs: Vec<_> = list_step_logs(&self.pool, instance.id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
            .into_iter()
            .filter(|l| l.step_id == current_step_id && l.status == "running")
            .collect();

        if let Some(log) = active_logs.first() {
            complete_step_log(&self.pool, &log.document_id, Some(step_output))
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
        }

        let next_step_id = resolve_next_step(current_step, &context);

        match next_step_id {
            Some(next_id) => {
                let next_step = steps.iter().find(|s| s.id == next_id);
                match next_step {
                    Some(ns) => {
                        update_instance_step(
                            &self.pool,
                            instance_id,
                            "running",
                            Some(&ns.id),
                            &context,
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
                            Some(&context),
                        )
                        .await
                        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                        self.get_instance(instance_id)
                            .await?
                            .ok_or_else(|| AppError::not_found("workflow instance"))
                    }
                    None => Err(AppError::Internal(anyhow::anyhow!(
                        "next step not found: {next_id}"
                    ))),
                }
            }
            None => {
                update_instance_step(&self.pool, instance_id, "completed", None, &context)
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

                self.get_instance(instance_id)
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
                .filter(|l| l.step_id == *step_id && l.status == "running")
                .collect();

            if let Some(log) = active_logs.first() {
                fail_step_log(&self.pool, &log.document_id, error)
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
            }
        }

        update_instance_step(&self.pool, instance_id, "failed", None, &json!({}))
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

        if instance.status != "running" {
            return Err(AppError::BadRequest(
                "only running instances can be cancelled".into(),
            ));
        }

        update_instance_step(&self.pool, instance_id, "cancelled", None, &json!({}))
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
        status: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WorkflowInstance>, i64)> {
        let def_id: Option<i64> = match definition_id {
            Some(did) => Some(self.get_workflow(did).await?.id),
            None => None,
        };
        crate::models::workflow::list_instances(&self.pool, def_id, status, page, page_size)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }

    /// 获取步骤日志
    pub async fn get_step_logs(
        &self,
        instance_id: &str,
    ) -> AppResult<Vec<crate::models::workflow::StepLog>> {
        let instance = self
            .get_instance(instance_id)
            .await?
            .ok_or_else(|| AppError::not_found("workflow instance"))?;
        list_step_logs(&self.pool, instance.id)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
    }
}

/// 验证步骤定义的合法性
fn validate_steps(steps: &[StepDef]) -> AppResult<()> {
    let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
    for step in steps {
        match &step.step_type {
            StepType::Branch => {
                let branches = step.next.as_array().ok_or_else(|| {
                    AppError::BadRequest("branch step must have array next".into())
                })?;
                for branch in branches {
                    let next_id = branch["step"].as_str().ok_or_else(|| {
                        AppError::BadRequest("branch must have 'step' field".into())
                    })?;
                    if !ids.contains(&next_id) {
                        return Err(AppError::BadRequest(format!(
                            "branch references unknown step: {next_id}"
                        )));
                    }
                }
            }
            StepType::Parallel => {
                let parallels = step.next.as_array().ok_or_else(|| {
                    AppError::BadRequest("parallel step must have array next".into())
                })?;
                for p in parallels {
                    let next_id = p.as_str().ok_or_else(|| {
                        AppError::BadRequest("parallel next must be step id string".into())
                    })?;
                    if !ids.contains(&next_id) {
                        return Err(AppError::BadRequest(format!(
                            "parallel references unknown step: {next_id}"
                        )));
                    }
                }
            }
            StepType::Task | StepType::Await | StepType::Delay => {
                if let Some(next_id) = step.next.as_str()
                    && !next_id.is_empty()
                    && !ids.contains(&next_id)
                {
                    return Err(AppError::BadRequest(format!(
                        "step references unknown next: {next_id}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// 根据步骤类型和 context 解析下一步
fn resolve_next_step(step: &StepDef, context: &serde_json::Value) -> Option<String> {
    match &step.step_type {
        StepType::Branch => {
            let branches = step.next.as_array()?;
            for branch in branches {
                if let Some(condition) = branch.get("condition")
                    && evaluate_condition(condition, context)
                {
                    return branch["step"].as_str().map(String::from);
                }
            }
            branches
                .iter()
                .find(|b| b.get("condition").is_none())
                .and_then(|b| b["step"].as_str().map(String::from))
        }
        StepType::Parallel => {
            let parallels = step.next.as_array()?;
            parallels.first().and_then(|p| p.as_str().map(String::from))
        }
        StepType::Task | StepType::Await | StepType::Delay => step.next.as_str().and_then(|s| {
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }),
    }
}

/// 评估条件表达式（简化版：支持字段等值比较）
fn evaluate_condition(condition: &serde_json::Value, context: &serde_json::Value) -> bool {
    if let Some(obj) = condition.as_object() {
        for (key, expected) in obj {
            let actual = context.get(key);
            match (actual, expected) {
                (Some(a), serde_json::Value::String(exp)) => {
                    if a.as_str() != Some(exp.as_str()) {
                        return false;
                    }
                }
                (Some(a), serde_json::Value::Number(exp)) => {
                    if a.as_f64() != exp.as_f64() {
                        return false;
                    }
                }
                (Some(a), serde_json::Value::Bool(exp)) => {
                    if a.as_bool() != Some(*exp) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        return true;
    }
    false
}

/// 将步骤输出合并到工作流 context
fn merge_output_into_context(context: &mut serde_json::Value, output: &serde_json::Value) {
    if let (Some(ctx_obj), Some(out_obj)) = (context.as_object_mut(), output.as_object()) {
        for (k, v) in out_obj {
            ctx_obj.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str, name: &str, step_type: StepType, next: serde_json::Value) -> StepDef {
        StepDef {
            id: id.to_string(),
            name: name.to_string(),
            step_type,
            config: json!({}),
            next,
            timeout_ms: 0,
        }
    }

    #[test]
    fn validate_steps_rejects_unknown_next() {
        let steps = vec![make_step("s1", "Step 1", StepType::Task, json!("s99"))];
        assert!(validate_steps(&steps).is_err());
    }

    #[test]
    fn validate_steps_accepts_valid_chain() {
        let steps = vec![
            make_step("s1", "Step 1", StepType::Task, json!("s2")),
            make_step("s2", "Step 2", StepType::Task, json!(null)),
        ];
        assert!(validate_steps(&steps).is_ok());
    }

    #[test]
    fn resolve_next_task_step() {
        let step = make_step("s1", "Review", StepType::Task, json!("s2"));
        let ctx = json!({});
        assert_eq!(resolve_next_step(&step, &ctx), Some("s2".to_string()));
    }

    #[test]
    fn resolve_next_task_step_empty_means_end() {
        let step = make_step("s1", "Final", StepType::Task, json!(""));
        let ctx = json!({});
        assert_eq!(resolve_next_step(&step, &ctx), None);
    }

    #[test]
    fn resolve_next_branch_step_matches_condition() {
        let step = make_step(
            "s1",
            "Decide",
            StepType::Branch,
            json!([
                {"condition": {"approved": true}, "step": "s2"},
                {"step": "s3"}
            ]),
        );
        let ctx = json!({"approved": true});
        assert_eq!(resolve_next_step(&step, &ctx), Some("s2".to_string()));
    }

    #[test]
    fn resolve_next_branch_step_falls_through() {
        let step = make_step(
            "s1",
            "Decide",
            StepType::Branch,
            json!([
                {"condition": {"approved": true}, "step": "s2"},
                {"step": "s3"}
            ]),
        );
        let ctx = json!({"approved": false});
        assert_eq!(resolve_next_step(&step, &ctx), Some("s3".to_string()));
    }

    #[test]
    fn evaluate_condition_equality() {
        let condition = json!({"status": "approved", "score": 10});
        let context = json!({"status": "approved", "score": 10, "name": "test"});
        assert!(evaluate_condition(&condition, &context));
    }

    #[test]
    fn evaluate_condition_mismatch() {
        let condition = json!({"status": "approved"});
        let context = json!({"status": "rejected"});
        assert!(!evaluate_condition(&condition, &context));
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
}

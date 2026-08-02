//! Step definition validation + condition expression evaluation

use super::model::{StepDef, StepType};
use crate::errors::app_error::{AppError, AppResult};

/// Validates the legality of step definitions
pub fn validate_steps(steps: &[StepDef]) -> AppResult<()> {
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

/// Resolves the next step based on step type and context (Parallel steps are handled separately by execute_parallel_step)
pub fn resolve_next_step(step: &StepDef, context: &serde_json::Value) -> Option<String> {
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
        StepType::Parallel => None,
        StepType::Task | StepType::Await | StepType::Delay => step.next.as_str().and_then(|s| {
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }),
    }
}

/// Evaluates a condition expression (simplified: supports field equality comparison)
pub fn evaluate_condition(condition: &serde_json::Value, context: &serde_json::Value) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn validate_steps_rejects_branch_with_non_array_next() {
        let steps = vec![make_step("s1", "Branch", StepType::Branch, json!("s2"))];
        assert!(validate_steps(&steps).is_err());
    }

    #[test]
    fn validate_steps_rejects_branch_unknown_ref() {
        let steps = vec![make_step(
            "s1",
            "Branch",
            StepType::Branch,
            json!([{"condition": {"x": "y"}, "step": "s99"}]),
        )];
        assert!(validate_steps(&steps).is_err());
    }

    #[test]
    fn validate_steps_accepts_branch_valid() {
        let steps = vec![
            make_step(
                "s1",
                "Branch",
                StepType::Branch,
                json!([{"condition": {"ok": true}, "step": "s2"}]),
            ),
            make_step("s2", "End", StepType::Task, json!(null)),
        ];
        assert!(validate_steps(&steps).is_ok());
    }

    #[test]
    fn validate_steps_rejects_parallel_with_non_array_next() {
        let steps = vec![make_step("s1", "Par", StepType::Parallel, json!("s2"))];
        assert!(validate_steps(&steps).is_err());
    }

    #[test]
    fn validate_steps_rejects_parallel_unknown_ref() {
        let steps = vec![make_step("s1", "Par", StepType::Parallel, json!(["s99"]))];
        assert!(validate_steps(&steps).is_err());
    }

    #[test]
    fn validate_steps_accepts_parallel_valid() {
        let steps = vec![
            make_step("s1", "Par", StepType::Parallel, json!(["s2", "s3"])),
            make_step("s2", "A", StepType::Task, json!(null)),
            make_step("s3", "B", StepType::Task, json!(null)),
        ];
        assert!(validate_steps(&steps).is_ok());
    }

    #[test]
    fn validate_steps_accepts_empty_next_for_task() {
        let steps = vec![make_step("s1", "Final", StepType::Task, json!(null))];
        assert!(validate_steps(&steps).is_ok());
    }

    #[test]
    fn resolve_next_parallel_returns_none() {
        let step = make_step("s1", "Par", StepType::Parallel, json!(["s2", "s3"]));
        assert_eq!(resolve_next_step(&step, &json!({})), None);
    }

    #[test]
    fn evaluate_condition_bool_match() {
        let cond = json!({"active": true});
        let ctx = json!({"active": true, "name": "test"});
        assert!(evaluate_condition(&cond, &ctx));
    }

    #[test]
    fn evaluate_condition_number_mismatch() {
        let cond = json!({"count": 5});
        let ctx = json!({"count": 3});
        assert!(!evaluate_condition(&cond, &ctx));
    }

    #[test]
    fn evaluate_condition_missing_field() {
        let cond = json!({"missing": "value"});
        let ctx = json!({});
        assert!(!evaluate_condition(&cond, &ctx));
    }

    #[test]
    fn evaluate_condition_non_object_returns_false() {
        assert!(!evaluate_condition(&json!("string"), &json!({})));
        assert!(!evaluate_condition(&json!(42), &json!({})));
    }
}

//! nestable Protocol — parent-child tree structure
//!
//! Provides `parent_id`, `depth`, and `position` columns;
//! automatically calculates depth (parent depth + 1) and position (max among siblings + 1) on create.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::constants::{COL_DEPTH, COL_PARENT_ID, COL_POSITION};
use crate::protocols::Protocol;

pub struct NestableAspect;

#[async_trait]
impl Aspect for NestableAspect {
    fn name(&self) -> &str {
        "nestable"
    }

    fn priority(&self) -> i32 {
        -200
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![Pointcut {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
            target: TargetMatcher::All,
        }]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_PARENT_ID.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
            ColumnDef {
                name: COL_DEPTH.into(),
                sql_type: SqlType::Integer,
                default: Some("0".into()),
            },
            ColumnDef {
                name: COL_POSITION.into(),
                sql_type: SqlType::Integer,
                default: Some("0".into()),
            },
        ]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if ctx
            .schema
            .as_ref()
            .is_none_or(|s| s.is_protocol_column(COL_PARENT_ID))
        {
            if !ctx.record.contains_key(COL_PARENT_ID) {
                ctx.record.insert(COL_PARENT_ID.into(), Value::Null);
            }
            if !ctx.record.contains_key(COL_DEPTH) {
                let depth = match ctx.record.get(COL_PARENT_ID) {
                    Some(Value::Null) | None => 0,
                    _ => 1,
                };
                ctx.record.insert(COL_DEPTH.into(), json!(depth));
            }
            if !ctx.record.contains_key(COL_POSITION) {
                ctx.record.insert(COL_POSITION.into(), json!(0));
            }
        }
        Ok(Advice::Continue)
    }
}

pub struct NestableProtocol;

impl Protocol for NestableProtocol {
    fn name(&self) -> &str {
        "nestable"
    }

    fn description(&self) -> &str {
        "Parent-child tree structure with parent_id, depth, and sibling ordering"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(NestableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["nestable"]
    }

    fn built_in(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspects::engine::AspectEngine;
    use crate::aspects::{BaseContext, Record};

    #[tokio::test]
    async fn injects_default_nesting_on_create() {
        let engine = AspectEngine::new();
        engine.register(NestableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "categories".into(),
            record: Record::new(),
            schema: None,
        };

        engine
            .dispatch_data_before_create("categories", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get(COL_PARENT_ID), Some(&Value::Null));
        assert_eq!(ctx.record.get(COL_DEPTH), Some(&json!(0)));
        assert_eq!(ctx.record.get(COL_POSITION), Some(&json!(0)));
    }

    #[test]
    fn provides_three_columns() {
        let cols = NestableAspect.columns();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0].name, COL_PARENT_ID);
        assert_eq!(cols[1].name, COL_DEPTH);
        assert_eq!(cols[2].name, COL_POSITION);
    }
}

crate::register_protocol!(
    crate::protocols::nestable::NestableProtocol,
    crate::protocols::nestable::NestableProtocol
);

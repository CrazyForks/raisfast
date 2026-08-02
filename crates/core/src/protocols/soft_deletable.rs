//! soft_deletable Protocol — mark deleted_at/deleted_by on delete instead of physical deletion
//!
//! Contains 1 Aspect: SoftDeletableAspect (priority = -300).
//! Sets soft_delete=true on before_delete and injects deleted_at/deleted_by.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeDeleteContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::constants::*;
use crate::protocols::Protocol;

pub struct SoftDeletableAspect;

#[async_trait]
impl Aspect for SoftDeletableAspect {
    fn name(&self) -> &str {
        "soft_deletable"
    }

    fn priority(&self) -> i32 {
        -300
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![Pointcut {
            layer: Layer::Data,
            operation: Operation::Delete,
            when: When::Before,
            target: TargetMatcher::All,
        }]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_DELETED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
            ColumnDef {
                name: COL_DELETED_BY.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
        ]
    }

    async fn on_data_before_delete(&self, ctx: &mut DataBeforeDeleteContext) -> AspectResult {
        if let Some(ref schema) = ctx.schema
            && !schema.implements_protocol("soft_deletable")
        {
            return Ok(Advice::Continue);
        }
        ctx.soft_delete = true;
        ctx.record
            .insert(COL_DELETED_AT.into(), json!(ctx.base.now));
        if let Some(user_int_id) = ctx.base.user_int_id {
            ctx.record.insert(COL_DELETED_BY.into(), json!(user_int_id));
        }
        Ok(Advice::Continue)
    }
}

pub struct SoftDeletableProtocol;

impl Protocol for SoftDeletableProtocol {
    fn name(&self) -> &str {
        "soft_deletable"
    }

    fn description(&self) -> &str {
        "Mark deleted_at on delete instead of physical deletion"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(SoftDeletableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["soft_delete"]
    }

    fn declaration(&self) -> crate::protocols::ProtocolDeclaration {
        crate::protocols::ProtocolDeclaration {
            query_filters: vec![(
                crate::constants::COL_DELETED_AT.to_string(),
                "IS NULL".to_string(),
            )],
            delete_strategy: crate::protocols::DeleteStrategy::Soft {
                column: crate::constants::COL_DELETED_AT.to_string(),
            },
            ..Default::default()
        }
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
    async fn sets_soft_delete_flag() {
        let engine = AspectEngine::new();
        engine.register(SoftDeletableAspect);

        let mut ctx = DataBeforeDeleteContext {
            base: BaseContext::new(
                Some("user-1".into()),
                "default".into(),
                "2026-01-01T00:00:00Z".into(),
            )
            .with_user_int_id(Some(1)),
            table: "articles".into(),
            record: Record::new(),
            soft_delete: false,
            schema: None,
        };

        engine
            .dispatch_data_before_delete("articles", &mut ctx)
            .await
            .unwrap();

        assert!(ctx.soft_delete);
        assert_eq!(
            ctx.record.get("deleted_at").unwrap(),
            &json!("2026-01-01T00:00:00Z")
        );
        assert_eq!(ctx.record.get("deleted_by").unwrap(), &json!(1));
    }

    #[tokio::test]
    async fn no_deleted_by_without_user() {
        let engine = AspectEngine::new();
        engine.register(SoftDeletableAspect);

        let mut ctx = DataBeforeDeleteContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: Record::new(),
            soft_delete: false,
            schema: None,
        };

        engine
            .dispatch_data_before_delete("articles", &mut ctx)
            .await
            .unwrap();

        assert!(ctx.soft_delete);
        assert!(ctx.record.contains_key("deleted_at"));
        assert!(!ctx.record.contains_key("deleted_by"));
    }

    #[tokio::test]
    async fn provides_columns() {
        let cols = SoftDeletableAspect.columns();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].name, "deleted_at");
        assert_eq!(cols[1].name, "deleted_by");
    }
}

crate::register_protocol!(
    crate::protocols::soft_deletable::SoftDeletableProtocol,
    crate::protocols::soft_deletable::SoftDeletableProtocol
);

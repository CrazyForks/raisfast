//! lockable Protocol — optimistic locking
//!
//! Provides a version column `version`; appends WHERE version = ? on UPDATE,
//! returns 409 Conflict when affected rows is 0.
//! The Aspect injects version = 1 on create.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::constants::COL_LOCK_VERSION;
use crate::protocols::{Protocol, ProtocolDeclaration};

pub struct LockableAspect;

#[async_trait]
impl Aspect for LockableAspect {
    fn name(&self) -> &str {
        "lockable"
    }

    fn priority(&self) -> i32 {
        -100
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
        vec![ColumnDef {
            name: COL_LOCK_VERSION.into(),
            sql_type: SqlType::Integer,
            default: Some("1".into()),
        }]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        let should_inject = ctx
            .schema
            .as_ref()
            .is_none_or(|s| s.is_protocol_column(COL_LOCK_VERSION));
        if should_inject {
            ctx.record.insert(COL_LOCK_VERSION.into(), json!(1));
        }
        Ok(Advice::Continue)
    }
}

pub struct LockableProtocol;

impl Protocol for LockableProtocol {
    fn name(&self) -> &str {
        "lockable"
    }

    fn description(&self) -> &str {
        "Optimistic locking; checks the version column on update to prevent concurrent overwrites"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(LockableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["optimistic_lock"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            lock_column: Some(COL_LOCK_VERSION.into()),
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
    async fn injects_version_on_create() {
        let engine = AspectEngine::new();
        engine.register(LockableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "posts".into(),
            record: Record::new(),
            schema: None,
        };

        engine
            .dispatch_data_before_create("posts", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get(COL_LOCK_VERSION).unwrap(), &json!(1));
    }

    #[tokio::test]
    async fn provides_version_column() {
        let cols = LockableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_LOCK_VERSION);
    }

    #[test]
    fn declaration_has_lock_column() {
        let decl = LockableProtocol.declaration();
        assert_eq!(decl.lock_column.as_deref(), Some(COL_LOCK_VERSION));
        assert!(decl.is_lockable());
    }
}

crate::register_protocol!(
    crate::protocols::lockable::LockableProtocol,
    crate::protocols::lockable::LockableProtocol
);

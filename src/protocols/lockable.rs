//! lockable Protocol — 乐观锁
//!
//! 提供版本列 `version`，UPDATE 时追加 WHERE version = ? 条件，
//! 受影响行数为 0 时返回 409 Conflict。
//! Aspect 在 create 时注入 version = 1。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
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
            name: "version".into(),
            sql_type: SqlType::Integer,
            default: Some("1".into()),
        }]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        ctx.record.insert("version".into(), json!(1));
        Ok(Advice::Continue)
    }
}

pub struct LockableProtocol;

impl Protocol for LockableProtocol {
    fn name(&self) -> &str {
        "lockable"
    }

    fn description(&self) -> &str {
        "乐观锁，更新时检查 version 列防止并发覆盖"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(LockableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["optimistic_lock"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            lock_column: Some("version".into()),
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

        assert_eq!(ctx.record.get("version").unwrap(), &json!(1));
    }

    #[tokio::test]
    async fn provides_version_column() {
        let cols = LockableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "version");
    }

    #[test]
    fn declaration_has_lock_column() {
        let decl = LockableProtocol.declaration();
        assert_eq!(decl.lock_column.as_deref(), Some("version"));
        assert!(decl.is_lockable());
    }
}

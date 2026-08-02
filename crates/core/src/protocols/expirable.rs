//! expirable Protocol — expiration time
//!
//! Provides an `expires_at` column; list queries automatically filter out expired records (expires_at IS NULL OR expires_at > now).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::constants::COL_EXPIRES_AT;
use crate::db::DbDriver;
use crate::protocols::{Protocol, ProtocolDeclaration};

pub struct ExpirableAspect;

#[async_trait]
impl Aspect for ExpirableAspect {
    fn name(&self) -> &str {
        "expirable"
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
        vec![ColumnDef {
            name: COL_EXPIRES_AT.into(),
            sql_type: SqlType::Timestamp,
            default: None,
        }]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if ctx
            .schema
            .as_ref()
            .is_none_or(|s| s.is_protocol_column(COL_EXPIRES_AT))
            && !ctx.record.contains_key(COL_EXPIRES_AT)
        {
            ctx.record.insert(COL_EXPIRES_AT.into(), Value::Null);
        }
        Ok(Advice::Continue)
    }
}

pub struct ExpirableProtocol;

impl Protocol for ExpirableProtocol {
    fn name(&self) -> &str {
        "expirable"
    }

    fn description(&self) -> &str {
        "Expiration time management; list queries automatically filter out expired records"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(ExpirableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["expirable"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            query_filters: vec![(
                COL_EXPIRES_AT.to_string(),
                format!("IS NULL OR expires_at > {}", crate::db::Driver::now_fn()),
            )],
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
    async fn injects_null_expires_at_on_create() {
        let engine = AspectEngine::new();
        engine.register(ExpirableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "coupons".into(),
            record: Record::new(),
            schema: None,
        };

        engine
            .dispatch_data_before_create("coupons", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get(COL_EXPIRES_AT), Some(&Value::Null));
    }

    #[test]
    fn provides_expires_at_column() {
        let cols = ExpirableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_EXPIRES_AT);
        assert_eq!(cols[0].sql_type, SqlType::Timestamp);
    }

    #[test]
    fn declaration_has_query_filter() {
        let decl = ExpirableProtocol.declaration();
        assert_eq!(decl.query_filters.len(), 1);
        assert_eq!(decl.query_filters[0].0, COL_EXPIRES_AT);
    }
}

crate::register_protocol!(
    crate::protocols::expirable::ExpirableProtocol,
    crate::protocols::expirable::ExpirableProtocol
);

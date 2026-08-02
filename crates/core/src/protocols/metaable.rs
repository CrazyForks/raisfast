//! metaable Protocol — dynamic JSON metadata
//!
//! Provides a `__meta` JSON column where users can freely read/write arbitrary key-value data
//! without adding table fields. System-internal data uses the `_sys` namespace for isolation.
//!
//! ```toml
//! implements = ["metaable"]
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::constants::COL_META;
use crate::protocols::Protocol;

pub struct MetaableAspect;

#[async_trait]
impl Aspect for MetaableAspect {
    fn name(&self) -> &str {
        "metaable"
    }

    fn priority(&self) -> i32 {
        1000
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
            name: COL_META.into(),
            sql_type: SqlType::Json,
            default: Some("'{}'".into()),
        }]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if ctx
            .schema
            .as_ref()
            .is_none_or(|s| s.is_protocol_column(COL_META))
            && !ctx.record.contains_key(COL_META)
        {
            ctx.record.insert(COL_META.into(), json!({}));
        }
        Ok(Advice::Continue)
    }
}

pub struct MetaableProtocol;

impl Protocol for MetaableProtocol {
    fn name(&self) -> &str {
        "metaable"
    }

    fn description(&self) -> &str {
        "Dynamic JSON metadata column; extend data without adding table fields"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(MetaableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["metaable"]
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
    async fn injects_empty_meta_on_create() {
        let engine = AspectEngine::new();
        engine.register(MetaableAspect);

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

        assert_eq!(ctx.record.get(COL_META), Some(&json!({})));
    }

    #[test]
    fn provides_meta_column() {
        let cols = MetaableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_META);
    }
}

crate::register_protocol!(
    crate::protocols::metaable::MetaableProtocol,
    crate::protocols::metaable::MetaableProtocol
);

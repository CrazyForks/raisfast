//! timestampable Protocol — automatically inject created_at / updated_at
//!
//! Built-in by default; automatically applies to all tables.
//! Contains 1 Aspect: TimestampableAspect (priority = -400).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, DataBeforeUpdateContext,
    Layer, Operation, Pointcut, SqlType, TargetMatcher, When,
};
use crate::constants::*;
use crate::protocols::Protocol;

pub struct TimestampableAspect;

#[async_trait]
impl Aspect for TimestampableAspect {
    fn name(&self) -> &str {
        "timestampable"
    }

    fn priority(&self) -> i32 {
        -400
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::Before,
                target: TargetMatcher::All,
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Update,
                when: When::Before,
                target: TargetMatcher::All,
            },
        ]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_CREATED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
            ColumnDef {
                name: COL_UPDATED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
        ]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        let schema = ctx.schema.as_ref();
        if schema.is_none_or(|s| s.is_protocol_column(COL_CREATED_AT)) {
            ctx.record
                .insert(COL_CREATED_AT.into(), json!(ctx.base.now));
        }
        if schema.is_none_or(|s| s.is_protocol_column(COL_UPDATED_AT)) {
            ctx.record
                .insert(COL_UPDATED_AT.into(), json!(ctx.base.now));
        }
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        if ctx
            .schema
            .as_ref()
            .is_none_or(|s| s.is_protocol_column(COL_UPDATED_AT))
        {
            ctx.new_record
                .insert(COL_UPDATED_AT.into(), json!(ctx.base.now));
        }
        Ok(Advice::Continue)
    }
}

pub struct TimestampableProtocol;

impl Protocol for TimestampableProtocol {
    fn name(&self) -> &str {
        "timestampable"
    }

    fn description(&self) -> &str {
        "Automatically injects timestamps on create and update"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(TimestampableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["track_timestamps"]
    }

    fn built_in(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspects::engine::AspectEngine;
    use crate::aspects::{BaseContext, DataBeforeUpdateContext, Record};

    async fn dispatch_create(
        engine: &AspectEngine,
        table: &str,
        ctx: &mut DataBeforeCreateContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        engine.dispatch_data_before_create(table, ctx).await
    }

    async fn dispatch_update(
        engine: &AspectEngine,
        table: &str,
        ctx: &mut DataBeforeUpdateContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        engine.dispatch_data_before_update(table, ctx).await
    }

    #[tokio::test]
    async fn injects_timestamps_on_create() {
        let engine = AspectEngine::new();
        engine.register(TimestampableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "2026-01-01T00:00:00Z".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        dispatch_create(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert_eq!(
            ctx.record.get("created_at").unwrap(),
            &json!("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            ctx.record.get("updated_at").unwrap(),
            &json!("2026-01-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn updates_updated_at_on_update() {
        let engine = AspectEngine::new();
        engine.register(TimestampableAspect);

        let mut ctx = DataBeforeUpdateContext {
            base: BaseContext::new(None, "default".into(), "2026-06-01T00:00:00Z".into()),
            table: "articles".into(),
            old_record: {
                let mut r = Record::new();
                r.insert("created_at".into(), json!("2026-01-01T00:00:00Z"));
                r.insert("updated_at".into(), json!("2026-01-01T00:00:00Z"));
                r
            },
            new_record: Record::new(),
            schema: None,
        };

        dispatch_update(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert_eq!(
            ctx.new_record.get("updated_at").unwrap(),
            &json!("2026-06-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn provides_timestamp_columns() {
        let cols = TimestampableAspect.columns();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].name, "created_at");
        assert_eq!(cols[1].name, "updated_at");
    }

    #[tokio::test]
    async fn update_does_not_modify_created_at() {
        let engine = AspectEngine::new();
        engine.register(TimestampableAspect);

        let mut new_record = Record::new();
        new_record.insert("title".into(), json!("updated"));

        let mut ctx = DataBeforeUpdateContext {
            base: BaseContext::new(None, "default".into(), "2026-06-01T00:00:00Z".into()),
            table: "articles".into(),
            old_record: {
                let mut r = Record::new();
                r.insert("created_at".into(), json!("2026-01-01T00:00:00Z"));
                r
            },
            new_record,
            schema: None,
        };

        dispatch_update(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert!(!ctx.new_record.contains_key("created_at"));
        assert!(ctx.new_record.contains_key("updated_at"));
    }

    #[tokio::test]
    async fn create_overwrites_existing_timestamps() {
        let engine = AspectEngine::new();
        engine.register(TimestampableAspect);

        let mut record = Record::new();
        record.insert("created_at".into(), json!("old-time"));
        record.insert("updated_at".into(), json!("old-time"));

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "2026-01-01T00:00:00Z".into()),
            table: "articles".into(),
            record,
            schema: None,
        };

        dispatch_create(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert_eq!(
            ctx.record.get("created_at").unwrap(),
            &json!("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            ctx.record.get("updated_at").unwrap(),
            &json!("2026-01-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn create_with_empty_record() {
        let engine = AspectEngine::new();
        engine.register(TimestampableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "2026-01-01T00:00:00Z".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        dispatch_create(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.len(), 2);
    }

    #[tokio::test]
    async fn two_pointcuts_registered() {
        let pcs = TimestampableAspect.pointcuts();
        assert_eq!(pcs.len(), 2);

        let ops: Vec<Operation> = pcs.iter().map(|p| p.operation).collect();
        assert!(ops.contains(&Operation::Create));
        assert!(ops.contains(&Operation::Update));

        let whens: Vec<When> = pcs.iter().map(|p| p.when).collect();
        assert!(whens.iter().all(|w| *w == When::Before));
    }

    #[tokio::test]
    async fn priority_is_negative_400() {
        assert_eq!(TimestampableAspect.priority(), -400);
    }
}

crate::register_protocol!(
    crate::protocols::timestampable::TimestampableProtocol,
    crate::protocols::timestampable::TimestampableProtocol
);

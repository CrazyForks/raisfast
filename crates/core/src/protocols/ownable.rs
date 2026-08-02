//! ownable Protocol — inject created_by on create, updated_by on update
//!
//! Built-in by default; automatically applies to all tables.
//! Contains 1 Aspect: OwnableAspect (priority = -500).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, DataBeforeUpdateContext,
    Layer, Operation, Pointcut, SqlType, TargetMatcher, When,
};
use crate::constants::*;
use crate::protocols::Protocol;

pub struct OwnableAspect;

#[async_trait]
impl Aspect for OwnableAspect {
    fn name(&self) -> &str {
        "ownable"
    }

    fn priority(&self) -> i32 {
        -500
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
                name: COL_CREATED_BY.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
            ColumnDef {
                name: COL_UPDATED_BY.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
        ]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if let Some(user_int_id) = ctx.base.user_int_id {
            let schema = ctx.schema.as_ref();
            if schema.is_none_or(|s| s.is_protocol_column(COL_CREATED_BY)) {
                ctx.record.insert(COL_CREATED_BY.into(), json!(user_int_id));
            }
            if schema.is_none_or(|s| s.is_protocol_column(COL_UPDATED_BY)) {
                ctx.record.insert(COL_UPDATED_BY.into(), json!(user_int_id));
            }
        }
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        if let Some(user_int_id) = ctx.base.user_int_id
            && ctx
                .schema
                .as_ref()
                .is_none_or(|s| s.is_protocol_column(COL_UPDATED_BY))
        {
            ctx.new_record
                .insert(COL_UPDATED_BY.into(), json!(user_int_id));
        }
        Ok(Advice::Continue)
    }
}

pub struct OwnableProtocol;

impl Protocol for OwnableProtocol {
    fn name(&self) -> &str {
        "ownable"
    }

    fn description(&self) -> &str {
        "Automatically injects the operator ID on create and update"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(OwnableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["track_owner"]
    }

    fn built_in(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspects::engine::AspectEngine;
    use crate::aspects::{BaseContext, DataBeforeCreateContext, Record};
    use crate::protocols::timestampable::TimestampableAspect;

    async fn dispatch_create(
        engine: &AspectEngine,
        table: &str,
        ctx: &mut DataBeforeCreateContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        engine.dispatch_data_before_create(table, ctx).await
    }

    #[tokio::test]
    async fn injects_created_by() {
        let engine = AspectEngine::new();
        engine.register(OwnableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(Some("user-123".into()), "default".into(), "now".into())
                .with_user_int_id(Some(42)),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        dispatch_create(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get("created_by").unwrap(), &json!(42));
        assert_eq!(ctx.record.get("updated_by").unwrap(), &json!(42));
    }

    #[tokio::test]
    async fn no_created_by_without_user() {
        let engine = AspectEngine::new();
        engine.register(OwnableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        dispatch_create(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert!(ctx.record.get("created_by").is_none());
        assert!(ctx.record.get("updated_by").is_none());
    }

    #[tokio::test]
    async fn provides_created_by_and_updated_by_columns() {
        let cols = OwnableAspect.columns();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].name, "created_by");
        assert_eq!(cols[1].name, "updated_by");
    }

    #[tokio::test]
    async fn combined_with_timestampable() {
        let engine = AspectEngine::new();
        engine.register(OwnableAspect);
        engine.register(TimestampableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(
                Some("user-1".into()),
                "default".into(),
                "2026-01-01T00:00:00Z".into(),
            )
            .with_user_int_id(Some(1)),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        dispatch_create(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get("created_by").unwrap(), &json!(1));
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
    async fn overwrites_existing_created_by() {
        let engine = AspectEngine::new();
        engine.register(OwnableAspect);

        let mut record = Record::new();
        record.insert("created_by".into(), json!("old-user"));

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(Some("new-user".into()), "default".into(), "now".into())
                .with_user_int_id(Some(99)),
            table: "articles".into(),
            record,
            schema: None,
        };

        dispatch_create(&engine, "articles", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get("created_by").unwrap(), &json!(99));
    }

    #[tokio::test]
    async fn updates_updated_by_on_update() {
        let engine = AspectEngine::new();
        engine.register(OwnableAspect);

        let mut new_record = Record::new();
        new_record.insert("title".into(), json!("updated"));

        let mut ctx = DataBeforeUpdateContext {
            base: BaseContext::new(Some("user-1".into()), "default".into(), "now".into())
                .with_user_int_id(Some(1)),
            table: "articles".into(),
            old_record: {
                let mut r = Record::new();
                r.insert("created_by".into(), json!("original-user"));
                r
            },
            new_record,
            schema: None,
        };

        let result: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_update("articles", &mut ctx)
            .await;
        result.unwrap();

        assert!(!ctx.new_record.contains_key("created_by"));
        assert_eq!(ctx.new_record.get("updated_by").unwrap(), &json!(1));
    }

    #[tokio::test]
    async fn dispatch_on_empty_table_name() {
        let engine = AspectEngine::new();
        engine.register(OwnableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(Some("user-1".into()), "default".into(), "now".into())
                .with_user_int_id(Some(1)),
            table: String::new(),
            record: Record::new(),
            schema: None,
        };

        dispatch_create(&engine, "", &mut ctx).await.unwrap();

        assert_eq!(ctx.record.get("created_by").unwrap(), &json!(1));
    }

    #[tokio::test]
    async fn multiple_dispatches_accumulate() {
        let engine = AspectEngine::new();
        engine.register(OwnableAspect);

        let mut ctx1 = DataBeforeCreateContext {
            base: BaseContext::new(Some("user-a".into()), "default".into(), "now".into())
                .with_user_int_id(Some(10)),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };
        dispatch_create(&engine, "articles", &mut ctx1)
            .await
            .unwrap();
        assert_eq!(ctx1.record.get("created_by").unwrap(), &json!(10));

        let mut ctx2 = DataBeforeCreateContext {
            base: BaseContext::new(Some("user-b".into()), "default".into(), "now".into())
                .with_user_int_id(Some(20)),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };
        dispatch_create(&engine, "articles", &mut ctx2)
            .await
            .unwrap();
        assert_eq!(ctx2.record.get("created_by").unwrap(), &json!(20));
    }

    #[tokio::test]
    async fn priority_is_negative_500() {
        assert_eq!(OwnableAspect.priority(), -500);
    }

    #[tokio::test]
    async fn pointcut_targets_all() {
        let pcs = OwnableAspect.pointcuts();
        assert_eq!(pcs.len(), 2);
        assert!(matches!(pcs[0].target, TargetMatcher::All));
        assert_eq!(pcs[0].layer, Layer::Data);
        assert_eq!(pcs[0].operation, Operation::Create);
        assert_eq!(pcs[0].when, When::Before);
        assert_eq!(pcs[1].operation, Operation::Update);
    }
}

crate::register_protocol!(
    crate::protocols::ownable::OwnableProtocol,
    crate::protocols::ownable::OwnableProtocol
);

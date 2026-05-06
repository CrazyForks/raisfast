//! sortable Protocol — 显式排序列
//!
//! 提供排序键列 `sort_key`，列表查询默认按 sort_key ASC 排序。
//! Aspect 在 create 时注入 sort_key = 0。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::protocols::{Protocol, ProtocolDeclaration, SortDir};

pub struct SortableAspect;

#[async_trait]
impl Aspect for SortableAspect {
    fn name(&self) -> &str {
        "sortable"
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
            name: "sort_key".into(),
            sql_type: SqlType::Integer,
            default: Some("0".into()),
        }]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if !ctx.record.contains_key("sort_key") {
            ctx.record.insert("sort_key".into(), json!(0));
        }
        Ok(Advice::Continue)
    }
}

pub struct SortableProtocol;

impl Protocol for SortableProtocol {
    fn name(&self) -> &str {
        "sortable"
    }

    fn description(&self) -> &str {
        "显式排序列，列表查询默认按 sort_key 排序"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(SortableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["sortable"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            default_sort: Some(("sort_key".into(), SortDir::Asc)),
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
    async fn injects_sort_key_on_create() {
        let engine = AspectEngine::new();
        engine.register(SortableAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "pages".into(),
            record: Record::new(),
            schema: None,
        };

        engine
            .dispatch_data_before_create("pages", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get("sort_key").unwrap(), &json!(0));
    }

    #[tokio::test]
    async fn does_not_overwrite_existing_sort_key() {
        let engine = AspectEngine::new();
        engine.register(SortableAspect);

        let mut record = Record::new();
        record.insert("sort_key".into(), json!(42));

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "pages".into(),
            record,
            schema: None,
        };

        engine
            .dispatch_data_before_create("pages", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get("sort_key").unwrap(), &json!(42));
    }

    #[tokio::test]
    async fn provides_sort_key_column() {
        let cols = SortableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "sort_key");
    }

    #[test]
    fn declaration_has_default_sort() {
        let decl = SortableProtocol.declaration();
        assert!(decl.default_sort.is_some());
        let (col, dir) = decl.default_sort.clone().unwrap();
        assert_eq!(col, "sort_key");
        assert_eq!(dir, SortDir::Asc);
        assert!(decl.is_sortable());
    }
}

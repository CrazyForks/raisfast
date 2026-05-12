//! sortable Protocol — explicit sort column
//!
//! Provides a sort key column `sort_key`; list queries default to sorting by sort_key Desc.
//! The Aspect injects sort_key = 0 on create.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataBeforeCreateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::constants::COL_SORT_KEY;
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
            name: COL_SORT_KEY.into(),
            sql_type: SqlType::Integer,
            default: Some("0".into()),
        }]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        let should_inject = ctx
            .schema
            .as_ref()
            .is_none_or(|s| s.is_protocol_column(COL_SORT_KEY));
        if should_inject && !ctx.record.contains_key(COL_SORT_KEY) {
            ctx.record.insert(COL_SORT_KEY.into(), json!(0));
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
        "Explicit sort column; list queries default to sorting by sort_key"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(SortableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["sortable"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            default_sort: Some((COL_SORT_KEY.into(), SortDir::Desc)),
            ..Default::default()
        }
    }

    fn apply_config(
        &self,
        config: &std::collections::HashMap<String, String>,
        decl: &mut ProtocolDeclaration,
        all_columns: &[&str],
    ) {
        if let Some(field) = config.get("field") {
            if !all_columns.contains(&field.as_str()) {
                tracing::warn!("sortable: field '{field}' not found, skipping default_sort");
                decl.default_sort = None;
                return;
            }
            let dir = config
                .get("direction")
                .map(|d| match d.to_lowercase().as_str() {
                    "desc" => SortDir::Desc,
                    _ => SortDir::Asc,
                })
                .unwrap_or(SortDir::Asc);
            decl.default_sort = Some((field.clone(), dir));
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

        assert_eq!(ctx.record.get(COL_SORT_KEY).unwrap(), &json!(0));
    }

    #[tokio::test]
    async fn does_not_overwrite_existing_sort_key() {
        let engine = AspectEngine::new();
        engine.register(SortableAspect);

        let mut record = Record::new();
        record.insert(COL_SORT_KEY.into(), json!(42));

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

        assert_eq!(ctx.record.get(COL_SORT_KEY).unwrap(), &json!(42));
    }

    #[tokio::test]
    async fn provides_sort_key_column() {
        let cols = SortableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_SORT_KEY);
    }

    #[test]
    fn declaration_has_default_sort() {
        let decl = SortableProtocol.declaration();
        assert!(decl.default_sort.is_some());
        let (col, dir) = decl.default_sort.clone().unwrap();
        assert_eq!(col, COL_SORT_KEY);
        assert_eq!(dir, SortDir::Desc);
        assert!(decl.is_sortable());
    }
}

crate::register_protocol!(
    crate::protocols::sortable::SortableProtocol,
    crate::protocols::sortable::SortableProtocol
);

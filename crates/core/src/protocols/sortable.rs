//! sortable Protocol — explicit sort column
//!
//! Provides a sort key column `sort_key`; list queries default to sorting by sort_key Desc.
//! The protocol injects sort_key = 0 on create.

use serde_json::{Value, json};

use crate::constants::COL_SORT_KEY;
use crate::db::sql_type::{ColumnDef, SqlType};
use crate::protocols::{HookCtx, Protocol, ProtocolDeclaration, SortDir};
use async_trait::async_trait;

pub struct SortableProtocol;

#[async_trait]
impl Protocol for SortableProtocol {
    fn name(&self) -> &str {
        "sortable"
    }

    fn description(&self) -> &str {
        "Explicit sort column; list queries default to sorting by sort_key"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_SORT_KEY.into(),
            sql_type: SqlType::Integer,
            default: Some("0".into()),
        }]
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

    async fn before_create(
        &self,
        record: &mut serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        let should_inject = ctx
            .schema
            .is_none_or(|s| s.is_protocol_column(COL_SORT_KEY));
        if should_inject && !record.contains_key(COL_SORT_KEY) {
            record.insert(COL_SORT_KEY.into(), json!(0));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> HookCtx<'static> {
        HookCtx {
            user_id: None,
            user_role: None,
            tenant_id: "default",
            now: "now",
            schema: None,
            pool: None,
        }
    }

    #[tokio::test]
    async fn injects_sort_key_on_create() {
        let protocol = SortableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx();

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get(COL_SORT_KEY).unwrap(), &json!(0));
    }

    #[tokio::test]
    async fn does_not_overwrite_existing_sort_key() {
        let protocol = SortableProtocol;
        let mut record = serde_json::Map::new();
        record.insert(COL_SORT_KEY.into(), json!(42));
        let ctx = ctx();

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get(COL_SORT_KEY).unwrap(), &json!(42));
    }

    #[tokio::test]
    async fn provides_sort_key_column() {
        let cols = SortableProtocol.columns();
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

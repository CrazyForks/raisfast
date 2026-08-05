//! nestable Protocol — parent-child tree structure
//!
//! Provides `parent_id`, `depth`, and `position` columns;
//! automatically calculates depth (parent depth + 1) and position (max among siblings + 1) on create.

use serde_json::{Value, json};

use crate::db::sql_type::{ColumnDef, SqlType};
use crate::constants::{COL_DEPTH, COL_PARENT_ID, COL_POSITION};
use crate::protocols::{HookCtx, Protocol};
use async_trait::async_trait;

pub struct NestableProtocol;

#[async_trait]
impl Protocol for NestableProtocol {
    fn name(&self) -> &str {
        "nestable"
    }

    fn description(&self) -> &str {
        "Parent-child tree structure with parent_id, depth, and sibling ordering"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_PARENT_ID.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
            ColumnDef {
                name: COL_DEPTH.into(),
                sql_type: SqlType::Integer,
                default: Some("0".into()),
            },
            ColumnDef {
                name: COL_POSITION.into(),
                sql_type: SqlType::Integer,
                default: Some("0".into()),
            },
        ]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["nestable"]
    }

    fn built_in(&self) -> bool {
        true
    }

    async fn before_create(
        &self,
        record: &mut serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if ctx
            .schema
            .is_none_or(|s| s.is_protocol_column(COL_PARENT_ID))
        {
            if !record.contains_key(COL_PARENT_ID) {
                record.insert(COL_PARENT_ID.into(), Value::Null);
            }
            if !record.contains_key(COL_DEPTH) {
                let depth = match record.get(COL_PARENT_ID) {
                    Some(Value::Null) | None => 0,
                    _ => 1,
                };
                record.insert(COL_DEPTH.into(), json!(depth));
            }
            if !record.contains_key(COL_POSITION) {
                record.insert(COL_POSITION.into(), json!(0));
            }
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
    async fn injects_default_nesting_on_create() {
        let protocol = NestableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx();

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get(COL_PARENT_ID), Some(&Value::Null));
        assert_eq!(record.get(COL_DEPTH), Some(&json!(0)));
        assert_eq!(record.get(COL_POSITION), Some(&json!(0)));
    }

    #[test]
    fn provides_three_columns() {
        let cols = NestableProtocol.columns();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0].name, COL_PARENT_ID);
        assert_eq!(cols[1].name, COL_DEPTH);
        assert_eq!(cols[2].name, COL_POSITION);
    }
}

crate::register_protocol!(
    crate::protocols::nestable::NestableProtocol,
    crate::protocols::nestable::NestableProtocol
);

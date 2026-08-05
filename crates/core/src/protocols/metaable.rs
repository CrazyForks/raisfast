//! metaable Protocol — dynamic JSON metadata
//!
//! Provides a `__meta` JSON column where users can freely read/write arbitrary key-value data
//! without adding table fields. System-internal data uses the `_sys` namespace for isolation.
//!
//! ```toml
//! implements = ["metaable"]
//! ```

use serde_json::{Value, json};

use crate::db::sql_type::{ColumnDef, SqlType};
use crate::constants::COL_META;
use crate::protocols::{HookCtx, Protocol};
use async_trait::async_trait;

pub struct MetaableProtocol;

#[async_trait]
impl Protocol for MetaableProtocol {
    fn name(&self) -> &str {
        "metaable"
    }

    fn description(&self) -> &str {
        "Dynamic JSON metadata column; extend data without adding table fields"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_META.into(),
            sql_type: SqlType::Json,
            default: Some("'{}'".into()),
        }]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["metaable"]
    }

    fn built_in(&self) -> bool {
        true
    }

    async fn before_create(
        &self,
        record: &mut serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if ctx.schema.is_none_or(|s| s.is_protocol_column(COL_META))
            && !record.contains_key(COL_META)
        {
            record.insert(COL_META.into(), json!({}));
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
    async fn injects_empty_meta_on_create() {
        let protocol = MetaableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx();

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get(COL_META), Some(&json!({})));
    }

    #[test]
    fn provides_meta_column() {
        let cols = MetaableProtocol.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_META);
    }
}

crate::register_protocol!(
    crate::protocols::metaable::MetaableProtocol,
    crate::protocols::metaable::MetaableProtocol
);

//! expirable Protocol — expiration time
//!
//! Provides an `expires_at` column; list queries automatically filter out expired records (expires_at IS NULL OR expires_at > now).

use serde_json::Value;

use crate::constants::COL_EXPIRES_AT;
use crate::db::DbDriver;
use crate::db::sql_type::{ColumnDef, SqlType};
use crate::protocols::{HookCtx, Protocol, ProtocolDeclaration};
use async_trait::async_trait;

pub struct ExpirableProtocol;

#[async_trait]
impl Protocol for ExpirableProtocol {
    fn name(&self) -> &str {
        "expirable"
    }

    fn description(&self) -> &str {
        "Expiration time management; list queries automatically filter out expired records"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_EXPIRES_AT.into(),
            sql_type: SqlType::Timestamp,
            default: None,
        }]
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

    async fn before_create(
        &self,
        record: &mut serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if ctx
            .schema
            .is_none_or(|s| s.is_protocol_column(COL_EXPIRES_AT))
            && !record.contains_key(COL_EXPIRES_AT)
        {
            record.insert(COL_EXPIRES_AT.into(), Value::Null);
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
    async fn injects_null_expires_at_on_create() {
        let protocol = ExpirableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx();

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get(COL_EXPIRES_AT), Some(&Value::Null));
    }

    #[test]
    fn provides_expires_at_column() {
        let cols = ExpirableProtocol.columns();
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

//! lockable Protocol — optimistic locking
//!
//! Provides a version column `version`; appends WHERE version = ? on UPDATE,
//! returns 409 Conflict when affected rows is 0.
//! The protocol injects version = 1 on create.

use serde_json::{Value, json};

use crate::db::sql_type::{ColumnDef, SqlType};
use crate::constants::COL_LOCK_VERSION;
use crate::protocols::{HookCtx, Protocol, ProtocolDeclaration};
use async_trait::async_trait;

pub struct LockableProtocol;

#[async_trait]
impl Protocol for LockableProtocol {
    fn name(&self) -> &str {
        "lockable"
    }

    fn description(&self) -> &str {
        "Optimistic locking; checks the version column on update to prevent concurrent overwrites"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_LOCK_VERSION.into(),
            sql_type: SqlType::Integer,
            default: Some("1".into()),
        }]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["optimistic_lock"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            lock_column: Some(COL_LOCK_VERSION.into()),
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
        let should_inject = ctx
            .schema
            .is_none_or(|s| s.is_protocol_column(COL_LOCK_VERSION));
        if should_inject {
            record.insert(COL_LOCK_VERSION.into(), json!(1));
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
    async fn injects_version_on_create() {
        let protocol = LockableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx();

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get(COL_LOCK_VERSION).unwrap(), &json!(1));
    }

    #[tokio::test]
    async fn provides_version_column() {
        let cols = LockableProtocol.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_LOCK_VERSION);
    }

    #[test]
    fn declaration_has_lock_column() {
        let decl = LockableProtocol.declaration();
        assert_eq!(decl.lock_column.as_deref(), Some(COL_LOCK_VERSION));
        assert!(decl.is_lockable());
    }
}

crate::register_protocol!(
    crate::protocols::lockable::LockableProtocol,
    crate::protocols::lockable::LockableProtocol
);

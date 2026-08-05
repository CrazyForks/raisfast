//! soft_deletable Protocol — mark deleted_at/deleted_by on delete instead of physical deletion
//!
//! Returns `DeleteAction::Soft` from `before_delete` to switch delete into an UPDATE.

use crate::db::sql_type::{ColumnDef, SqlType};
use crate::constants::*;
use crate::protocols::{DeleteAction, HookCtx, Protocol, ProtocolDeclaration};
use async_trait::async_trait;

pub struct SoftDeletableProtocol;

#[async_trait]
impl Protocol for SoftDeletableProtocol {
    fn name(&self) -> &str {
        "soft_deletable"
    }

    fn description(&self) -> &str {
        "Mark deleted_at on delete instead of physical deletion"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_DELETED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
            ColumnDef {
                name: COL_DELETED_BY.into(),
                sql_type: SqlType::BigInt,
                default: None,
            },
        ]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["soft_delete"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            query_filters: vec![(COL_DELETED_AT.to_string(), "IS NULL".to_string())],
            delete_strategy: crate::protocols::DeleteStrategy::Soft {
                column: COL_DELETED_AT.to_string(),
            },
            ..Default::default()
        }
    }

    fn built_in(&self) -> bool {
        true
    }

    async fn before_delete(
        &self,
        _record: &serde_json::Map<String, serde_json::Value>,
        ctx: &mut HookCtx<'_>,
    ) -> anyhow::Result<DeleteAction> {
        if ctx
            .schema
            .is_none_or(|s| s.is_protocol_column(COL_DELETED_AT))
        {
            Ok(DeleteAction::Soft {
                deleted_at: ctx.now.to_string(),
                deleted_by: ctx.user_id,
            })
        } else {
            Ok(DeleteAction::Hard)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_user(now: &'static str, user_id: Option<i64>) -> HookCtx<'static> {
        HookCtx {
            user_id,
            user_role: None,
            tenant_id: "default",
            now,
            schema: None,
            pool: None,
        }
    }

    #[tokio::test]
    async fn returns_soft_delete_action() {
        let protocol = SoftDeletableProtocol;
        let record = serde_json::Map::new();
        let mut ctx = ctx_with_user("2026-01-01T00:00:00Z", Some(1));

        let action = protocol.before_delete(&record, &mut ctx).await.unwrap();

        match action {
            DeleteAction::Soft {
                deleted_at,
                deleted_by,
            } => {
                assert_eq!(deleted_at, "2026-01-01T00:00:00Z");
                assert_eq!(deleted_by, Some(1));
            }
            DeleteAction::Hard => panic!("expected Soft"),
        }
    }

    #[tokio::test]
    async fn soft_delete_without_user() {
        let protocol = SoftDeletableProtocol;
        let record = serde_json::Map::new();
        let mut ctx = ctx_with_user("now", None);

        let action = protocol.before_delete(&record, &mut ctx).await.unwrap();

        match action {
            DeleteAction::Soft {
                deleted_at,
                deleted_by,
            } => {
                assert_eq!(deleted_at, "now");
                assert_eq!(deleted_by, None);
            }
            DeleteAction::Hard => panic!("expected Soft"),
        }
    }

    #[tokio::test]
    async fn provides_columns() {
        let cols = SoftDeletableProtocol.columns();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].name, "deleted_at");
        assert_eq!(cols[1].name, "deleted_by");
    }
}

crate::register_protocol!(
    crate::protocols::soft_deletable::SoftDeletableProtocol,
    crate::protocols::soft_deletable::SoftDeletableProtocol
);

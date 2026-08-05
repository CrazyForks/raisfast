//! ownable Protocol — inject created_by on create, updated_by on update
//!
//! Built-in by default; automatically applies to all tables.

use serde_json::{Value, json};

use crate::db::sql_type::{ColumnDef, SqlType};
use crate::constants::*;
use crate::protocols::{HookCtx, Protocol};
use async_trait::async_trait;

pub struct OwnableProtocol;

#[async_trait]
impl Protocol for OwnableProtocol {
    fn name(&self) -> &str {
        "ownable"
    }

    fn description(&self) -> &str {
        "Automatically injects the operator ID on create and update"
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

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["track_owner"]
    }

    fn built_in(&self) -> bool {
        true
    }

    async fn before_create(
        &self,
        record: &mut serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if let Some(user_int_id) = ctx.user_id {
            if ctx
                .schema
                .is_none_or(|s| s.is_protocol_column(COL_CREATED_BY))
            {
                record.insert(COL_CREATED_BY.into(), json!(user_int_id));
            }
            if ctx
                .schema
                .is_none_or(|s| s.is_protocol_column(COL_UPDATED_BY))
            {
                record.insert(COL_UPDATED_BY.into(), json!(user_int_id));
            }
        }
        Ok(())
    }

    async fn before_update(
        &self,
        new_record: &mut serde_json::Map<String, Value>,
        _old_record: &serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if let Some(user_int_id) = ctx.user_id
            && ctx
                .schema
                .is_none_or(|s| s.is_protocol_column(COL_UPDATED_BY))
        {
            new_record.insert(COL_UPDATED_BY.into(), json!(user_int_id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::timestampable::TimestampableProtocol;

    fn ctx_with_user(user_id: Option<i64>) -> HookCtx<'static> {
        HookCtx {
            user_id,
            user_role: None,
            tenant_id: "default",
            now: "now",
            schema: None,
            pool: None,
        }
    }

    #[tokio::test]
    async fn injects_created_by() {
        let protocol = OwnableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx_with_user(Some(42));

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get("created_by").unwrap(), &json!(42));
        assert_eq!(record.get("updated_by").unwrap(), &json!(42));
    }

    #[tokio::test]
    async fn no_created_by_without_user() {
        let protocol = OwnableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx_with_user(None);

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert!(record.get("created_by").is_none());
        assert!(record.get("updated_by").is_none());
    }

    #[tokio::test]
    async fn provides_created_by_and_updated_by_columns() {
        let cols = OwnableProtocol.columns();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].name, "created_by");
        assert_eq!(cols[1].name, "updated_by");
    }

    #[tokio::test]
    async fn combined_with_timestampable() {
        let ownable = OwnableProtocol;
        let timestampable = TimestampableProtocol;

        let mut record = serde_json::Map::new();
        let ctx = HookCtx {
            user_id: Some(1),
            user_role: None,
            tenant_id: "default",
            now: "2026-01-01T00:00:00Z",
            schema: None,
            pool: None,
        };

        ownable.before_create(&mut record, &ctx).await.unwrap();
        timestampable
            .before_create(&mut record, &ctx)
            .await
            .unwrap();

        assert_eq!(record.get("created_by").unwrap(), &json!(1));
        assert_eq!(
            record.get("created_at").unwrap(),
            &json!("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            record.get("updated_at").unwrap(),
            &json!("2026-01-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn overwrites_existing_created_by() {
        let protocol = OwnableProtocol;
        let mut record = serde_json::Map::new();
        record.insert("created_by".into(), json!("old-user"));
        let ctx = ctx_with_user(Some(99));

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.get("created_by").unwrap(), &json!(99));
    }

    #[tokio::test]
    async fn updates_updated_by_on_update() {
        let protocol = OwnableProtocol;
        let mut new_record = serde_json::Map::new();
        new_record.insert("title".into(), json!("updated"));
        let old_record = serde_json::Map::new();
        let ctx = ctx_with_user(Some(1));

        protocol
            .before_update(&mut new_record, &old_record, &ctx)
            .await
            .unwrap();

        assert!(!new_record.contains_key("created_by"));
        assert_eq!(new_record.get("updated_by").unwrap(), &json!(1));
    }

    #[tokio::test]
    async fn multiple_dispatches_accumulate() {
        let protocol = OwnableProtocol;

        let mut record1 = serde_json::Map::new();
        let ctx1 = ctx_with_user(Some(10));
        protocol.before_create(&mut record1, &ctx1).await.unwrap();
        assert_eq!(record1.get("created_by").unwrap(), &json!(10));

        let mut record2 = serde_json::Map::new();
        let ctx2 = ctx_with_user(Some(20));
        protocol.before_create(&mut record2, &ctx2).await.unwrap();
        assert_eq!(record2.get("created_by").unwrap(), &json!(20));
    }
}

crate::register_protocol!(
    crate::protocols::ownable::OwnableProtocol,
    crate::protocols::ownable::OwnableProtocol
);

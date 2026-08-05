//! timestampable Protocol — automatically inject created_at / updated_at
//!
//! Built-in by default; automatically applies to all tables.

use serde_json::{Value, json};

use crate::constants::*;
use crate::db::sql_type::{ColumnDef, SqlType};
use crate::protocols::{HookCtx, Protocol};
use async_trait::async_trait;

pub struct TimestampableProtocol;

#[async_trait]
impl Protocol for TimestampableProtocol {
    fn name(&self) -> &str {
        "timestampable"
    }

    fn description(&self) -> &str {
        "Automatically injects timestamps on create and update"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                name: COL_CREATED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
            ColumnDef {
                name: COL_UPDATED_AT.into(),
                sql_type: SqlType::Timestamp,
                default: None,
            },
        ]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["track_timestamps"]
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
            .is_none_or(|s| s.is_protocol_column(COL_CREATED_AT))
        {
            record.insert(COL_CREATED_AT.into(), json!(ctx.now));
        }
        if ctx
            .schema
            .is_none_or(|s| s.is_protocol_column(COL_UPDATED_AT))
        {
            record.insert(COL_UPDATED_AT.into(), json!(ctx.now));
        }
        Ok(())
    }

    async fn before_update(
        &self,
        new_record: &mut serde_json::Map<String, Value>,
        _old_record: &serde_json::Map<String, Value>,
        ctx: &HookCtx<'_>,
    ) -> anyhow::Result<()> {
        if ctx
            .schema
            .is_none_or(|s| s.is_protocol_column(COL_UPDATED_AT))
        {
            new_record.insert(COL_UPDATED_AT.into(), json!(ctx.now));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_now(now: &'static str) -> HookCtx<'static> {
        HookCtx {
            user_id: None,
            user_role: None,
            tenant_id: "default",
            now,
            schema: None,
            pool: None,
        }
    }

    #[tokio::test]
    async fn injects_timestamps_on_create() {
        let protocol = TimestampableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx_now("2026-01-01T00:00:00Z");

        protocol.before_create(&mut record, &ctx).await.unwrap();

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
    async fn updates_updated_at_on_update() {
        let protocol = TimestampableProtocol;
        let mut new_record = serde_json::Map::new();
        let old_record = serde_json::Map::new();
        let ctx = ctx_now("2026-06-01T00:00:00Z");

        protocol
            .before_update(&mut new_record, &old_record, &ctx)
            .await
            .unwrap();

        assert_eq!(
            new_record.get("updated_at").unwrap(),
            &json!("2026-06-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn provides_timestamp_columns() {
        let cols = TimestampableProtocol.columns();
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].name, "created_at");
        assert_eq!(cols[1].name, "updated_at");
    }

    #[tokio::test]
    async fn update_does_not_modify_created_at() {
        let protocol = TimestampableProtocol;
        let mut new_record = serde_json::Map::new();
        new_record.insert("title".into(), json!("updated"));
        let old_record = serde_json::Map::new();
        let ctx = ctx_now("2026-06-01T00:00:00Z");

        protocol
            .before_update(&mut new_record, &old_record, &ctx)
            .await
            .unwrap();

        assert!(!new_record.contains_key("created_at"));
        assert!(new_record.contains_key("updated_at"));
    }

    #[tokio::test]
    async fn create_overwrites_existing_timestamps() {
        let protocol = TimestampableProtocol;
        let mut record = serde_json::Map::new();
        record.insert("created_at".into(), json!("old-time"));
        record.insert("updated_at".into(), json!("old-time"));
        let ctx = ctx_now("2026-01-01T00:00:00Z");

        protocol.before_create(&mut record, &ctx).await.unwrap();

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
    async fn create_with_empty_record() {
        let protocol = TimestampableProtocol;
        let mut record = serde_json::Map::new();
        let ctx = ctx_now("2026-01-01T00:00:00Z");

        protocol.before_create(&mut record, &ctx).await.unwrap();

        assert_eq!(record.len(), 2);
    }
}

crate::register_protocol!(
    crate::protocols::timestampable::TimestampableProtocol,
    crate::protocols::timestampable::TimestampableProtocol
);

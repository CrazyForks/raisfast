//! versionable Protocol — automatically save old version snapshots on update
//!
//! Declares `snapshot_before_update` and `revision_routes`;
//! repository.update() proactively creates snapshots based on the declaration.

use crate::types::snowflake_id::SnowflakeId;
use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::{
    Advice, Aspect, AspectResult, ColumnDef, DataAfterUpdateContext, Layer, Operation, Pointcut,
    SqlType, TargetMatcher, When,
};
use crate::constants::*;
use crate::protocols::Protocol;

pub struct VersionableAspect;

#[async_trait]
impl Aspect for VersionableAspect {
    fn name(&self) -> &str {
        "versionable"
    }

    fn priority(&self) -> i32 {
        500
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![Pointcut {
            layer: Layer::Data,
            operation: Operation::Update,
            when: When::After,
            target: TargetMatcher::All,
        }]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_VERSION.into(),
            sql_type: SqlType::Integer,
            default: Some("1".into()),
        }]
    }

    async fn on_data_after_update(&self, _ctx: &mut DataAfterUpdateContext) -> AspectResult {
        Ok(Advice::Continue)
    }
}

pub struct VersionableProtocol;

impl Protocol for VersionableProtocol {
    fn name(&self) -> &str {
        "versionable"
    }

    fn description(&self) -> &str {
        "Automatically saves old version snapshots on update"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(VersionableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["versioning"]
    }

    fn declaration(&self) -> crate::protocols::ProtocolDeclaration {
        crate::protocols::ProtocolDeclaration {
            snapshot_before_update: true,
            revision_routes: true,
            ..Default::default()
        }
    }

    fn on_after_delete(
        &self,
        pool: &crate::db::pool::Pool,
        content_type_singular: &str,
        record_id: SnowflakeId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), anyhow::Error>> + Send + '_>>
    {
        let pool = pool.clone();
        let singular = content_type_singular.to_string();
        let id = record_id;
        Box::pin(async move {
            let _ = crate::models::content_revision::delete_revisions(&pool, &singular, id).await;
            Ok(())
        })
    }

    fn register_routes(
        &self,
        router: axum::Router<crate::AppState>,
        plural: &str,
        admin_prefix: &str,
    ) -> axum::Router<crate::AppState> {
        router
            .route(
                &format!("{admin_prefix}/{plural}/{{id}}/revisions"),
                axum::routing::get(crate::handlers::content_revision::list_revisions),
            )
            .route(
                &format!("{admin_prefix}/{plural}/{{id}}/revisions/{{revision_id}}"),
                axum::routing::get(crate::handlers::content_revision::get_revision),
            )
            .route(
                &format!("{admin_prefix}/{plural}/{{id}}/revisions/{{revision_id}}/restore"),
                axum::routing::post(crate::handlers::content_revision::restore_revision),
            )
            .route(
                &format!("{admin_prefix}/{plural}/{{id}}/revisions/{{rev_a}}/diff/{{rev_b}}"),
                axum::routing::get(crate::handlers::content_revision::diff_revisions),
            )
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
    async fn provides_version_column() {
        let cols = VersionableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_VERSION);
        assert_eq!(cols[0].sql_type, SqlType::Integer);
    }

    #[tokio::test]
    async fn pointcut_targets_after_update() {
        let pcs = VersionableAspect.pointcuts();
        assert_eq!(pcs.len(), 1);
        assert_eq!(pcs[0].operation, Operation::Update);
        assert_eq!(pcs[0].when, When::After);
    }

    #[tokio::test]
    async fn skips_without_pool() {
        let engine = AspectEngine::new();
        engine.register(VersionableAspect);

        let mut ctx = DataAfterUpdateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            old_record: Record::new(),
            new_record: Record::new(),
            schema: None,
        };

        let result: Result<_, _> = engine
            .dispatch_data_after_update("articles", &mut ctx)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn skips_without_id() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let engine = AspectEngine::new();
        engine.register(VersionableAspect);

        let mut ctx = DataAfterUpdateContext {
            base: BaseContext::new(None, "default".into(), "now".into())
                .with_pool(crate::db::pool::Pool::from(pool)),
            table: "articles".into(),
            old_record: Record::new(),
            new_record: Record::new(),
            schema: None,
        };

        let result: Result<_, _> = engine
            .dispatch_data_after_update("articles", &mut ctx)
            .await;
        assert!(result.is_ok());
    }
}

crate::register_protocol!(
    crate::protocols::versionable::VersionableProtocol,
    crate::protocols::versionable::VersionableProtocol
);

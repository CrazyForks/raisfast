//! versionable Protocol — automatically save old version snapshots on update
//!
//! Declares `snapshot_before_update` and `revision_routes`;
//! repository.update() proactively creates snapshots based on the declaration.

use crate::db::sql_type::{ColumnDef, SqlType};
use crate::constants::COL_VERSION;
use crate::protocols::{Protocol, ProtocolDeclaration};
use crate::types::snowflake_id::SnowflakeId;
use async_trait::async_trait;

pub struct VersionableProtocol;

#[async_trait]
impl Protocol for VersionableProtocol {
    fn name(&self) -> &str {
        "versionable"
    }

    fn description(&self) -> &str {
        "Automatically saves old version snapshots on update"
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_VERSION.into(),
            sql_type: SqlType::BigInt,
            default: Some("1".into()),
        }]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["versioning"]
    }

    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration {
            snapshot_before_update: true,
            revision_routes: true,
            ..Default::default()
        }
    }

    fn on_after_delete(
        &self,
        pool: &crate::db::pool::Pool,
        content_type_key: &str,
        record_id: SnowflakeId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), anyhow::Error>> + Send + '_>>
    {
        let pool = pool.clone();
        let key = content_type_key.to_string();
        let id = record_id;
        Box::pin(async move {
            let _ = crate::models::content_revision::delete_revisions(&pool, &key, id).await;
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

    #[tokio::test]
    async fn provides_version_column() {
        let cols = VersionableProtocol.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_VERSION);
        assert_eq!(cols[0].sql_type, SqlType::BigInt);
    }

    #[test]
    fn declaration_has_snapshot_and_routes() {
        let decl = VersionableProtocol.declaration();
        assert!(decl.snapshot_before_update);
        assert!(decl.revision_routes);
    }
}

crate::register_protocol!(
    crate::protocols::versionable::VersionableProtocol,
    crate::protocols::versionable::VersionableProtocol
);

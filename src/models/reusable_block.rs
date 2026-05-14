//! Reusable block model and database queries

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::commands::{CreateReusableBlockCmd, UpdateReusableBlockCmd};
use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReusableBlock {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub name: String,
    pub block_type: String,
    pub content: String,
    pub description: Option<String>,
    pub created_by: Option<i64>,
    pub updated_by: Option<i64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(ReusableBlock {
    required { id, document_id, name, block_type, content, created_at, updated_at }
    optional { description, created_by, updated_by }
});

pub async fn find_reusable_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<ReusableBlock>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM reusable_blocks WHERE id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, ReusableBlock>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

pub async fn find_reusable_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<ReusableBlock>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!(
        "SELECT * FROM reusable_blocks WHERE document_id = {}{filter}",
        ph(1)
    );
    let mut q = sqlx::query_as::<_, ReusableBlock>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

pub async fn list_reusable(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<ReusableBlock>> {
    let filter = tenant_filter_ph(tenant_id, 1);
    let sql = format!("SELECT * FROM reusable_blocks WHERE 1=1{filter} ORDER BY name ASC");
    let mut q = sqlx::query_as::<_, ReusableBlock>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_all(pool).await?)
}

pub async fn create_reusable(
    pool: &crate::db::Pool,
    cmd: &CreateReusableBlockCmd,
    tenant_id: Option<&str>,
) -> AppResult<ReusableBlock> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    match tenant_id {
        Some(tid) => {
            let vals = (1..=10).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO reusable_blocks (document_id, tenant_id, name, block_type, content, description, created_by, updated_by, created_at, updated_at) VALUES ({vals})"
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(&cmd.name)
                .bind(&cmd.block_type)
                .bind(&cmd.content)
                .bind(&cmd.description)
                .bind(cmd.created_by)
                .bind(cmd.created_by)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
        }
        None => {
            let vals = (1..=9).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO reusable_blocks (document_id, name, block_type, content, description, created_by, updated_by, created_at, updated_at) VALUES ({vals})"
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(&cmd.name)
                .bind(&cmd.block_type)
                .bind(&cmd.content)
                .bind(&cmd.description)
                .bind(cmd.created_by)
                .bind(cmd.created_by)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
        }
    }

    find_reusable_by_document_id(pool, &document_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))
}

pub async fn update_reusable(
    pool: &crate::db::Pool,
    cmd: &UpdateReusableBlockCmd,
    tenant_id: Option<&str>,
) -> AppResult<ReusableBlock> {
    let now = crate::utils::tz::now_utc();
    let mut idx = 1;
    let mut sets = vec![format!("updated_at = {}", ph(1))];

    if cmd.updated_by.is_some() {
        idx += 1;
        sets.push(format!("updated_by = {}", ph(idx)));
    }
    if cmd.name.is_some() {
        idx += 1;
        sets.push(format!("name = {}", ph(idx)));
    }
    if cmd.block_type.is_some() {
        idx += 1;
        sets.push(format!("block_type = {}", ph(idx)));
    }
    if cmd.content.is_some() {
        idx += 1;
        sets.push(format!("content = {}", ph(idx)));
    }
    if cmd.description.is_some() {
        idx += 1;
        sets.push(format!("description = {}", ph(idx)));
    }

    idx += 1;
    let id_ph = ph(idx);
    let tf = tenant_filter_ph(tenant_id, idx + 1);
    let sql = format!(
        "UPDATE reusable_blocks SET {} WHERE id = {id_ph}{tf}",
        sets.join(", ")
    );

    let mut q = sqlx::query(&sql);
    q = q.bind(now);
    if let Some(v) = cmd.updated_by {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.name {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.block_type {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.content {
        q = q.bind(v);
    }
    if let Some(ref v) = cmd.description {
        q = q.bind(v);
    }
    q = q.bind(cmd.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }

    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "reusable_block")?;

    find_reusable_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))
}

pub async fn delete_reusable(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("DELETE FROM reusable_blocks WHERE id = {}{filter}", ph(1));
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "reusable_block")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: "blockuser".to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
            },
            None,
        )
        .await
        .unwrap();
        user.id
    }

    #[sqlx::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        let block = create_reusable(
            &pool,
            &CreateReusableBlockCmd {
                name: "Block".to_string(),
                block_type: "text".to_string(),
                content: "[]".to_string(),
                description: None,
                created_by: Some(uid),
            },
            None,
        )
        .await
        .unwrap();
        let found = find_reusable_by_id(&pool, block.id, None).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Block");
    }

    #[sqlx::test]
    async fn find_by_document_id_test() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        let block = create_reusable(
            &pool,
            &CreateReusableBlockCmd {
                name: "Block".to_string(),
                block_type: "text".to_string(),
                content: "[]".to_string(),
                description: None,
                created_by: Some(uid),
            },
            None,
        )
        .await
        .unwrap();
        let found = super::find_reusable_by_document_id(&pool, &block.document_id, None)
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, block.id);
    }

    #[sqlx::test]
    async fn list_reusable_test() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        for i in 0..3 {
            create_reusable(
                &pool,
                &CreateReusableBlockCmd {
                    name: format!("Block{i}"),
                    block_type: "text".to_string(),
                    content: "[]".to_string(),
                    description: None,
                    created_by: Some(uid),
                },
                None,
            )
            .await
            .unwrap();
        }
        let list = super::list_reusable(&pool, None).await.unwrap();
        assert!(list.len() >= 3);
    }

    #[sqlx::test]
    async fn update_changes_name() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        let block = create_reusable(
            &pool,
            &CreateReusableBlockCmd {
                name: "Block".to_string(),
                block_type: "text".to_string(),
                content: "[]".to_string(),
                description: None,
                created_by: Some(uid),
            },
            None,
        )
        .await
        .unwrap();
        let updated = update_reusable(
            &pool,
            &UpdateReusableBlockCmd {
                id: block.id,
                name: Some("Updated".to_string()),
                block_type: None,
                content: None,
                description: None,
                updated_by: Some(uid),
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "Updated");
    }

    #[sqlx::test]
    async fn delete_removes_block() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        let block = create_reusable(
            &pool,
            &CreateReusableBlockCmd {
                name: "Block".to_string(),
                block_type: "text".to_string(),
                content: "[]".to_string(),
                description: None,
                created_by: Some(uid),
            },
            None,
        )
        .await
        .unwrap();
        delete_reusable(&pool, block.id, None).await.unwrap();
        let found = find_reusable_by_id(&pool, block.id, None).await.unwrap();
        assert!(found.is_none());
    }
}

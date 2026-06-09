//! Reusable block model and database queries

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::commands::{CreateReusableBlockCmd, UpdateReusableBlockCmd};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ReusableBlock {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub name: String,
    pub block_type: String,
    pub content: String,
    pub description: Option<String>,
    pub created_by: Option<SnowflakeId>,
    pub updated_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_reusable_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<ReusableBlock>> {
    Ok(
        raisfast_derive::crud_find!(pool, "reusable_blocks", ReusableBlock, where: ("id", id), tenant: tenant_id)?,
    )
}

pub async fn list_reusable(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<ReusableBlock>> {
    let result: Vec<ReusableBlock> = raisfast_derive::crud_list!(pool, "reusable_blocks", ReusableBlock, order_by: "name ASC", tenant: tenant_id)?;
    Ok(result)
}

pub async fn create_reusable(
    pool: &crate::db::Pool,
    cmd: &CreateReusableBlockCmd,
    tenant_id: Option<&str>,
) -> AppResult<ReusableBlock> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "reusable_blocks",
        [
            "id" => id,
            "name" => &cmd.name,
            "block_type" => &cmd.block_type,
            "content" => &cmd.content,
            "description" => &cmd.description,
            "created_by" => cmd.created_by,
            "updated_by" => cmd.created_by,
            "created_at" => now,
            "updated_at" => now
        ],
        tenant: tenant_id
    )?;

    find_reusable_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))
}

pub async fn update_reusable(
    pool: &crate::db::Pool,
    cmd: &UpdateReusableBlockCmd,
    tenant_id: Option<&str>,
) -> AppResult<ReusableBlock> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool, "reusable_blocks",
        bind: ["updated_at" => now],
        optional: ["updated_by" => cmd.updated_by, "name" => cmd.name, "block_type" => cmd.block_type, "content" => cmd.content, "description" => cmd.description],
        where: ("id", cmd.id),
        tenant: tenant_id
    )?;

    find_reusable_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))
}

pub async fn delete_reusable(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(pool, "reusable_blocks", where: ("id", id), tenant: tenant_id)?;
    AppError::expect_affected(&result, "reusable_block")
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: "blockuser".to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
            },
            None,
        )
        .await
        .unwrap();
        *user.id
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

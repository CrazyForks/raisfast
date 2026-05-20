//! Reusable block service layer

use crate::commands::{CreateReusableBlockCmd, UpdateReusableBlockCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::page;
use crate::models::reusable_block;
use crate::types::snowflake_id::SnowflakeId;

fn validate_blocks_json(blocks: &str) -> AppResult<Vec<page::PageBlock>> {
    serde_json::from_str(blocks)
        .map_err(|e| AppError::BadRequest(format!("invalid blocks JSON: {e}")))
}

pub async fn list_reusable(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<reusable_block::ReusableBlock>> {
    reusable_block::list_reusable(pool, auth.tenant_id()).await
}

pub async fn get_reusable(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    auth: &AuthUser,
) -> AppResult<Option<reusable_block::ReusableBlock>> {
    reusable_block::find_reusable_by_id(pool, id, auth.tenant_id()).await
}

pub async fn create_reusable(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    name: &str,
    block_type: &str,
    content: &str,
    description: Option<&str>,
) -> AppResult<reusable_block::ReusableBlock> {
    validate_blocks_json(content)?;

    reusable_block::create_reusable(
        pool,
        &CreateReusableBlockCmd {
            name: name.to_string(),
            block_type: block_type.to_string(),
            content: content.to_string(),
            description: description.map(|s| s.to_string()),
            created_by: auth.user_id(),
        },
        auth.tenant_id(),
    )
    .await
}

pub async fn update_reusable(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    auth: &AuthUser,
    name: Option<&str>,
    block_type: Option<&str>,
    content: Option<&str>,
    description: Option<&str>,
) -> AppResult<reusable_block::ReusableBlock> {
    if let Some(c) = content {
        validate_blocks_json(c)?;
    }
    let block = reusable_block::find_reusable_by_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    reusable_block::update_reusable(
        pool,
        &UpdateReusableBlockCmd {
            id: block.id,
            name: name.map(|s| s.to_string()),
            block_type: block_type.map(|s| s.to_string()),
            content: content.map(|s| s.to_string()),
            description: description.map(|s| s.to_string()),
            updated_by: auth.user_id(),
        },
        auth.tenant_id(),
    )
    .await
}

pub async fn delete_reusable(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    auth: &AuthUser,
) -> AppResult<()> {
    let block = reusable_block::find_reusable_by_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    reusable_block::delete_reusable(pool, block.id, auth.tenant_id()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_blocks_json_valid_empty() {
        let blocks = validate_blocks_json("[]").unwrap();
        assert!(blocks.is_empty());
    }

    #[test]
    fn validate_blocks_json_valid_block() {
        let json = r#"[{"type":"richtext","content":"block content"}]"#;
        let blocks = validate_blocks_json(json).unwrap();
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn validate_blocks_json_invalid() {
        assert!(validate_blocks_json("{bad").is_err());
    }

    #[test]
    fn validate_blocks_json_wrong_type() {
        assert!(validate_blocks_json(r#"[123]"#).is_err());
    }
}

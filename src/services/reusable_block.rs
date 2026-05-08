//! 可复用块服务层

use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::page;
use crate::models::reusable_block;

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
    id: &str,
    auth: &AuthUser,
) -> AppResult<Option<reusable_block::ReusableBlock>> {
    reusable_block::find_reusable_by_document_id(pool, id, auth.tenant_id()).await
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
        name,
        block_type,
        content,
        description,
        auth.user_int_id(),
        auth.tenant_id(),
    )
    .await
}

pub async fn update_reusable(
    pool: &crate::db::Pool,
    id: &str,
    auth: &AuthUser,
    name: Option<&str>,
    block_type: Option<&str>,
    content: Option<&str>,
    description: Option<&str>,
) -> AppResult<reusable_block::ReusableBlock> {
    if let Some(c) = content {
        validate_blocks_json(c)?;
    }
    let block = reusable_block::find_reusable_by_document_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    reusable_block::update_reusable(
        pool,
        block.id,
        name,
        block_type,
        content,
        description,
        auth.user_int_id(),
        auth.tenant_id(),
    )
    .await
}

pub async fn delete_reusable(pool: &crate::db::Pool, id: &str, auth: &AuthUser) -> AppResult<()> {
    let block = reusable_block::find_reusable_by_document_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    reusable_block::delete_reusable(pool, block.id, auth.tenant_id()).await
}

//! 页面服务层
//!
//! 提供页面和可复用块的完整 CRUD 业务逻辑，包括 slug 生成、状态管理和 block 校验。

use slug::slugify;

use crate::commands::{CreatePageCmd, UpdatePageCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::page;

fn validate_blocks_json(blocks: &str) -> AppResult<Vec<page::PageBlock>> {
    serde_json::from_str(blocks)
        .map_err(|e| AppError::BadRequest(format!("invalid blocks JSON: {e}")))
}

pub async fn list_published(
    pool: &crate::db::Pool,
    page_num: i64,
    page_size: i64,
    auth: &AuthUser,
) -> AppResult<(Vec<page::Page>, i64)> {
    page::list_published(pool, page_num, page_size, auth.tenant_id()).await
}

pub async fn get_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    auth: &AuthUser,
) -> AppResult<page::Page> {
    page::find_by_slug(pool, slug, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn get_by_id(pool: &crate::db::Pool, id: &str, auth: &AuthUser) -> AppResult<page::Page> {
    page::find_by_document_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn list_all(
    pool: &crate::db::Pool,
    page_num: i64,
    page_size: i64,
    status: Option<&str>,
    auth: &AuthUser,
) -> AppResult<(Vec<page::Page>, i64)> {
    page::list_all(pool, page_num, page_size, status, auth.tenant_id()).await
}

pub async fn create_page(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    cmd: CreatePageCmd,
) -> AppResult<page::Page> {
    if let Some(ref blocks) = cmd.blocks {
        validate_blocks_json(blocks)?;
    }

    let status = if cmd.status.is_empty() {
        "draft".to_string()
    } else {
        cmd.status.clone()
    };

    page::create(
        pool,
        &cmd.title,
        &cmd.slug,
        cmd.content.as_deref(),
        cmd.blocks.as_deref(),
        cmd.meta_title.as_deref(),
        cmd.meta_description.as_deref(),
        cmd.og_image.as_deref(),
        &cmd.template,
        cmd.parent_id,
        cmd.sort_order,
        &status,
        cmd.created_by,
        cmd.cover_image.as_deref(),
        auth.tenant_id(),
    )
    .await
}

pub async fn update_page(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    document_id: &str,
    cmd: UpdatePageCmd,
) -> AppResult<page::Page> {
    if let Some(ref blocks) = cmd.blocks {
        validate_blocks_json(blocks)?;
    }

    let existing = page::find_by_document_id(pool, document_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("page"))?;

    page::update(
        pool,
        existing.id,
        cmd.title.as_deref(),
        cmd.slug.as_deref(),
        cmd.content.as_deref(),
        cmd.blocks.as_deref(),
        cmd.meta_title.as_deref(),
        cmd.meta_description.as_deref(),
        cmd.og_image.as_deref(),
        cmd.template.as_deref(),
        cmd.parent_id,
        cmd.sort_order,
        cmd.status.as_deref(),
        cmd.cover_image.as_deref(),
        cmd.updated_by,
        auth.tenant_id(),
    )
    .await
}

pub async fn delete_page(pool: &crate::db::Pool, id: &str, auth: &AuthUser) -> AppResult<()> {
    let p = page::find_by_document_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("page"))?;
    page::delete(pool, p.id, auth.tenant_id()).await
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: &str,
    status: &str,
    auth: &AuthUser,
) -> AppResult<page::Page> {
    let valid = ["draft", "published", "archived"];
    if !valid.contains(&status) {
        return Err(AppError::BadRequest(format!("invalid status: {status}")));
    }
    let p = page::find_by_document_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("page"))?;
    page::update_status(pool, p.id, status, auth.user_int_id(), auth.tenant_id()).await
}

pub async fn reorder(
    pool: &crate::db::Pool,
    items: Vec<(String, i64)>,
    auth: &AuthUser,
) -> AppResult<()> {
    let mut resolved = Vec::new();
    for (doc_id, sort_order) in items {
        let p = page::find_by_document_id(pool, &doc_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("page"))?;
        resolved.push((p.id, sort_order));
    }
    page::reorder(pool, &resolved, auth.tenant_id()).await
}

pub async fn sitemap(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<(String, Option<String>)>> {
    page::list_sitemap(pool, auth.tenant_id()).await
}

// ── 可复用块 ──

pub async fn list_reusable(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<page::ReusableBlock>> {
    page::list_reusable(pool, auth.tenant_id()).await
}

pub async fn get_reusable(
    pool: &crate::db::Pool,
    id: &str,
    auth: &AuthUser,
) -> AppResult<Option<page::ReusableBlock>> {
    page::find_reusable_by_document_id(pool, id, auth.tenant_id()).await
}

pub async fn create_reusable(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    name: &str,
    block_type: &str,
    content: &str,
    description: Option<&str>,
) -> AppResult<page::ReusableBlock> {
    validate_blocks_json(content)?;

    page::create_reusable(
        pool,
        name,
        block_type,
        content,
        description,
        auth.user_id(),
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
) -> AppResult<page::ReusableBlock> {
    if let Some(c) = content {
        validate_blocks_json(c)?;
    }
    let block = page::find_reusable_by_document_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    page::update_reusable(
        pool,
        block.id,
        name,
        block_type,
        content,
        description,
        auth.user_id(),
        auth.tenant_id(),
    )
    .await
}

pub async fn delete_reusable(pool: &crate::db::Pool, id: &str, auth: &AuthUser) -> AppResult<()> {
    let block = page::find_reusable_by_document_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("reusable_block"))?;
    page::delete_reusable(pool, block.id, auth.tenant_id()).await
}

pub fn generate_slug(title: &str) -> String {
    slugify(title)
}

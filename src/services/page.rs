//! 页面服务层
//!
//! 提供页面和可复用块的完整 CRUD 业务逻辑，包括 slug 生成、状态管理和 block 校验。

use slug::slugify;

use crate::aspects::engine::AspectEngine;
use crate::commands::{CreatePageCmd, UpdatePageCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::models::page;
use crate::services::aspect_dispatch::{AspectDispatch, id_record};

fn validate_blocks_json(blocks: &str) -> AppResult<Vec<page::PageBlock>> {
    serde_json::from_str(blocks)
        .map_err(|e| AppError::BadRequest(format!("invalid blocks JSON: {e}")))
}

pub async fn list_published(
    pool: &crate::db::Pool,
    page_num: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<page::Page>, i64)> {
    page::list_published(pool, page_num, page_size, tenant_id).await
}

pub async fn get_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<page::Page> {
    page::find_by_slug(pool, slug, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn get_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<page::Page> {
    page::find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("page"))
}

pub async fn list_all(
    pool: &crate::db::Pool,
    page_num: i64,
    page_size: i64,
    status: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<page::Page>, i64)> {
    page::list_all(pool, page_num, page_size, status, tenant_id).await
}

pub async fn create_page(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: Option<&str>,
    cmd: CreatePageCmd,
    tenant_id: Option<&str>,
) -> AppResult<page::Page> {
    if let Some(ref blocks) = cmd.blocks {
        validate_blocks_json(blocks)?;
    }

    let status = if cmd.status.is_empty() {
        "draft".to_string()
    } else {
        cmd.status.clone()
    };

    let (id, _) = crate::utils::id::new_id_and_timestamp();

    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "pages",
        user_id,
        tenant_id,
    };
    dsp.before_create(id_record("")).await?;
    let result = page::create(
        pool,
        &id,
        &cmd.title,
        &cmd.slug,
        cmd.content.as_deref(),
        cmd.blocks.as_deref(),
        cmd.meta_title.as_deref(),
        cmd.meta_description.as_deref(),
        cmd.og_image.as_deref(),
        &cmd.template,
        cmd.parent_id.as_deref(),
        cmd.sort_order,
        &status,
        &cmd.author_id,
        cmd.cover_image.as_deref(),
        tenant_id,
    )
    .await;
    dsp.after_create(id_record(&id)).await;
    result
}

pub async fn update_page(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: Option<&str>,
    cmd: UpdatePageCmd,
    tenant_id: Option<&str>,
) -> AppResult<page::Page> {
    if let Some(ref blocks) = cmd.blocks {
        validate_blocks_json(blocks)?;
    }

    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "pages",
        user_id,
        tenant_id,
    };
    dsp.before_update(id_record(&cmd.id), id_record(&cmd.id))
        .await?;
    let result = page::update(
        pool,
        &cmd.id,
        cmd.title.as_deref(),
        cmd.slug.as_deref(),
        cmd.content.as_deref(),
        cmd.blocks.as_deref(),
        cmd.meta_title.as_deref(),
        cmd.meta_description.as_deref(),
        cmd.og_image.as_deref(),
        cmd.template.as_deref(),
        cmd.parent_id.as_ref().map(|opt| opt.as_deref()),
        cmd.sort_order,
        cmd.status.as_deref(),
        cmd.cover_image.as_deref(),
        tenant_id,
    )
    .await;
    dsp.after_update(id_record(&cmd.id)).await;
    result
}

pub async fn delete_page(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: Option<&str>,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "pages",
        user_id,
        tenant_id,
    };
    dsp.before_delete(id_record(id)).await?;
    let result = page::delete(pool, id, tenant_id).await;
    dsp.after_delete().await;
    result
}

pub async fn update_status(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: Option<&str>,
    id: &str,
    status: &str,
    tenant_id: Option<&str>,
) -> AppResult<page::Page> {
    let valid = ["draft", "published", "archived"];
    if !valid.contains(&status) {
        return Err(AppError::BadRequest(format!("invalid status: {status}")));
    }
    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "pages",
        user_id,
        tenant_id,
    };
    dsp.before_update(id_record(id), id_record(id)).await?;
    let result = page::update_status(pool, id, status, tenant_id).await;
    dsp.after_update(id_record(id)).await;
    result
}

pub async fn reorder(
    pool: &crate::db::Pool,
    items: Vec<(String, i64)>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    page::reorder(pool, &items, tenant_id).await
}

pub async fn sitemap(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<(String, Option<String>)>> {
    page::list_sitemap(pool, tenant_id).await
}

// ── 可复用块 ──

pub async fn list_reusable(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<page::ReusableBlock>> {
    page::list_reusable(pool, tenant_id).await
}

pub async fn get_reusable(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<page::ReusableBlock>> {
    page::find_reusable_by_id(pool, id, tenant_id).await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_reusable(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: Option<&str>,
    name: &str,
    block_type: &str,
    content: &str,
    description: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<page::ReusableBlock> {
    validate_blocks_json(content)?;
    let (id, _) = crate::utils::id::new_id_and_timestamp();

    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "reusable_blocks",
        user_id,
        tenant_id,
    };
    dsp.before_create(id_record("")).await?;
    let result =
        page::create_reusable(pool, &id, name, block_type, content, description, tenant_id).await;
    dsp.after_create(id_record(&id)).await;
    result
}

#[allow(clippy::too_many_arguments)]
pub async fn update_reusable(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: Option<&str>,
    id: &str,
    name: Option<&str>,
    block_type: Option<&str>,
    content: Option<&str>,
    description: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<page::ReusableBlock> {
    if let Some(c) = content {
        validate_blocks_json(c)?;
    }
    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "reusable_blocks",
        user_id,
        tenant_id,
    };
    dsp.before_update(id_record(id), id_record(id)).await?;
    let result =
        page::update_reusable(pool, id, name, block_type, content, description, tenant_id).await;
    dsp.after_update(id_record(id)).await;
    result
}

pub async fn delete_reusable(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: Option<&str>,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "reusable_blocks",
        user_id,
        tenant_id,
    };
    dsp.before_delete(id_record(id)).await?;
    let result = page::delete_reusable(pool, id, tenant_id).await;
    dsp.after_delete().await;
    result
}

pub fn generate_slug(title: &str) -> String {
    slugify(title)
}

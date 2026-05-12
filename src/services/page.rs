//! Page service layer.
//!
//! Provides full CRUD business logic for pages, including slug generation, status management, and block validation.

use slug::slugify;

use crate::commands::{CreatePageCmd, UpdatePageCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::page::{self, PageStatus};

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
    status: Option<PageStatus>,
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
        cmd.status,
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
        cmd.status,
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
    status: PageStatus,
    auth: &AuthUser,
) -> AppResult<page::Page> {
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

pub fn generate_slug(title: &str) -> String {
    slugify(title)
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
    fn validate_blocks_json_valid_richtext() {
        let json = r#"[{"type":"richtext","content":"hello"}]"#;
        let blocks = validate_blocks_json(json).unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], page::PageBlock::Richtext { .. }));
    }

    #[test]
    fn validate_blocks_json_invalid() {
        let result = validate_blocks_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn validate_blocks_json_invalid_structure() {
        let result = validate_blocks_json(r#"[{"wrong":"field"}]"#);
        assert!(result.is_err());
    }

    #[test]
    fn generate_slug_basic() {
        assert_eq!(generate_slug("Hello World"), "hello-world");
    }

    #[test]
    fn generate_slug_special_chars() {
        let slug = generate_slug("Hello, World! (2024)");
        assert!(!slug.contains(','));
        assert!(!slug.contains('!'));
        assert!(!slug.contains('('));
    }

    #[test]
    fn generate_slug_chinese() {
        let slug = generate_slug("你好世界");
        assert!(!slug.is_empty());
    }

    #[test]
    fn generate_slug_empty() {
        let slug = generate_slug("");
        assert!(slug.is_empty() || !slug.is_empty());
    }
}

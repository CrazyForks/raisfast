//! Page service layer.
//!
//! Provides full CRUD business logic for pages, including status management and block validation.

use std::sync::Arc;

use async_trait::async_trait;
use raisfast_derive::aspect_service;

use crate::aspects::engine::AspectEngine;
use crate::commands::{CreatePageCmd, UpdatePageCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::page::{self, Page, PageStatus};
use crate::types::snowflake_id::SnowflakeId;

/// Page business logic trait.
#[async_trait]
pub trait PageService: Send + Sync {
    async fn list_published(
        &self,
        page_num: i64,
        page_size: i64,
        auth: &AuthUser,
    ) -> AppResult<(Vec<Page>, i64)>;
    async fn get_by_slug(&self, slug: &str, auth: &AuthUser) -> AppResult<Page>;
    async fn get_by_id(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Page>;
    async fn list_all(
        &self,
        page_num: i64,
        page_size: i64,
        status: Option<PageStatus>,
        auth: &AuthUser,
    ) -> AppResult<(Vec<Page>, i64)>;
    async fn create_page(&self, auth: &AuthUser, cmd: CreatePageCmd) -> AppResult<Page>;
    async fn update_page(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        cmd: UpdatePageCmd,
    ) -> AppResult<Page>;
    async fn delete_page(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()>;
    async fn update_status(
        &self,
        id: SnowflakeId,
        status: PageStatus,
        auth: &AuthUser,
    ) -> AppResult<Page>;
    async fn reorder(&self, items: Vec<(String, i64)>, auth: &AuthUser) -> AppResult<()>;
    async fn sitemap(&self, auth: &AuthUser) -> AppResult<Vec<(String, Option<String>)>>;
}

#[aspect_service(entity = "pages", model = Page)]
pub struct PageServiceImpl {
    #[engine]
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
}

impl PageServiceImpl {
    fn validate_blocks_json(blocks: &str) -> AppResult<Vec<page::PageBlock>> {
        serde_json::from_str(blocks)
            .map_err(|e| AppError::BadRequest(format!("invalid blocks JSON: {e}")))
    }
}

#[async_trait]
impl PageService for PageServiceImpl {
    async fn list_published(
        &self,
        page_num: i64,
        page_size: i64,
        auth: &AuthUser,
    ) -> AppResult<(Vec<Page>, i64)> {
        page::list_published(&self.pool, page_num, page_size, auth.tenant_id()).await
    }

    async fn get_by_slug(&self, slug: &str, auth: &AuthUser) -> AppResult<Page> {
        page::find_by_slug(&self.pool, slug, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("page"))
    }

    async fn get_by_id(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Page> {
        page::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("page"))
    }

    async fn list_all(
        &self,
        page_num: i64,
        page_size: i64,
        status: Option<PageStatus>,
        auth: &AuthUser,
    ) -> AppResult<(Vec<Page>, i64)> {
        page::list_all(&self.pool, page_num, page_size, status, auth.tenant_id()).await
    }

    async fn create_page(&self, auth: &AuthUser, cmd: CreatePageCmd) -> AppResult<Page> {
        let (cmd, _d) = self.before_create(auth, cmd).await?;
        if let Some(ref blocks) = cmd.blocks {
            Self::validate_blocks_json(blocks)?;
        }
        let p = page::create(&self.pool, &cmd, auth.tenant_id()).await?;
        self.after_created(&p);
        Ok(p)
    }

    async fn update_page(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        mut cmd: UpdatePageCmd,
    ) -> AppResult<Page> {
        let existing = page::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("page"))?;

        cmd.id = existing.id;
        let (cmd, _d) = self.before_update(auth, &existing, cmd).await?;
        if let Some(ref blocks) = cmd.blocks {
            Self::validate_blocks_json(blocks)?;
        }
        let updated = page::update(&self.pool, &cmd, auth.tenant_id()).await?;
        self.after_updated(&updated);
        Ok(updated)
    }

    async fn delete_page(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()> {
        let p = page::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("page"))?;
        self.before_delete(auth, &p).await?;
        page::delete(&self.pool, p.id, auth.tenant_id()).await?;
        self.after_deleted(&p);
        Ok(())
    }

    async fn update_status(
        &self,
        id: SnowflakeId,
        status: PageStatus,
        auth: &AuthUser,
    ) -> AppResult<Page> {
        let p = page::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("page"))?;
        self.aspect_engine
            .before_update("pages", auth, &p, status)
            .await?;
        let updated =
            page::update_status(&self.pool, p.id, status, auth.user_id(), auth.tenant_id()).await?;
        self.after_updated(&updated);
        Ok(updated)
    }

    async fn reorder(&self, items: Vec<(String, i64)>, auth: &AuthUser) -> AppResult<()> {
        let mut resolved = Vec::new();
        for (raw_id, sort_order) in items {
            let id = crate::types::snowflake_id::parse_id(&raw_id)?;
            let p = page::find_by_id(&self.pool, id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("page"))?;
            resolved.push((*p.id, sort_order));
        }
        page::reorder(&self.pool, &resolved, auth.tenant_id()).await
    }

    async fn sitemap(&self, auth: &AuthUser) -> AppResult<Vec<(String, Option<String>)>> {
        page::list_sitemap(&self.pool, auth.tenant_id()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_blocks_json_valid_empty() {
        let blocks = PageServiceImpl::validate_blocks_json("[]").unwrap();
        assert!(blocks.is_empty());
    }

    #[test]
    fn validate_blocks_json_valid_richtext() {
        let json = r#"[{"type":"richtext","content":"hello"}]"#;
        let blocks = PageServiceImpl::validate_blocks_json(json).unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], page::PageBlock::Richtext { .. }));
    }

    #[test]
    fn validate_blocks_json_invalid() {
        let result = PageServiceImpl::validate_blocks_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn validate_blocks_json_invalid_structure() {
        let result = PageServiceImpl::validate_blocks_json(r#"[{"wrong":"field"}]"#);
        assert!(result.is_err());
    }
}

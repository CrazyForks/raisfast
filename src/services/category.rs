//! 分类服务。

use slug::slugify;

use crate::commands::{CreateCategoryCmd, UpdateCategoryCmd};
use crate::dto::{CreateCategoryRequest, UpdateCategoryRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::repositories::CategoryRepository;

pub async fn create_category(
    category_repo: &dyn CategoryRepository,
    auth: &AuthUser,
    req: CreateCategoryRequest,
) -> AppResult<crate::models::category::Category> {
    let slug = slugify(&req.name);
    let parent_id = if let Some(ref doc_id) = req.parent_id {
        if doc_id.parse::<i64>().is_ok() {
            doc_id.parse::<i64>().ok()
        } else {
            category_repo
                .find_by_document_id(doc_id, auth.tenant_id())
                .await?
                .map(|c| c.id)
        }
    } else {
        None
    };
    category_repo
        .create(
            CreateCategoryCmd {
                name: req.name,
                slug,
                description: req.description,
                parent_id,
                sort_order: req.sort_order.unwrap_or(0),
            },
            auth.tenant_id(),
            auth.user_int_id(),
        )
        .await
}

pub async fn update_category(
    category_repo: &dyn CategoryRepository,
    auth: &AuthUser,
    id: &str,
    req: UpdateCategoryRequest,
) -> AppResult<crate::models::category::Category> {
    let existing = category_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("category"))?;
    let new_slug = req.name.as_ref().map(slugify).unwrap_or(existing.slug);

    let parent_id = if let Some(ref doc_id) = req.parent_id {
        if doc_id.parse::<i64>().is_ok() {
            doc_id.parse::<i64>().ok()
        } else {
            category_repo
                .find_by_document_id(doc_id, auth.tenant_id())
                .await?
                .map(|c| c.id)
        }
    } else {
        None
    };
    category_repo
        .update(
            UpdateCategoryCmd {
                id: existing.id,
                name: req.name,
                slug: Some(new_slug),
                description: req.description,
                parent_id,
                sort_order: req.sort_order,
            },
            auth.tenant_id(),
            auth.user_int_id(),
        )
        .await
}

pub async fn delete_category(
    category_repo: &dyn CategoryRepository,
    id: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    let existing = category_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("category"))?;
    category_repo.delete(existing.id, auth.tenant_id()).await?;
    Ok(())
}

pub async fn list_categories(
    category_repo: &dyn CategoryRepository,
    auth: &AuthUser,
) -> AppResult<Vec<crate::models::category::Category>> {
    category_repo.find_all(auth.tenant_id()).await
}

pub async fn list_categories_paginated(
    category_repo: &dyn CategoryRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<crate::models::category::Category>, i64)> {
    category_repo
        .find_paginated(auth.tenant_id(), page, page_size)
        .await
}

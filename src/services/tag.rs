//! 标签服务。

use slug::slugify;

use crate::errors::app_error::{AppError, AppResult};
use crate::dto::CreateTagRequest;
use crate::middleware::auth::AuthUser;
use crate::repositories::TagRepository;

pub async fn create_tag(
    tag_repo: &dyn TagRepository,
    auth: &AuthUser,
    req: CreateTagRequest,
) -> AppResult<crate::models::tag::Tag> {
    let slug = slugify(&req.name);
    tag_repo
        .create(&req.name, &slug, auth.tenant_id(), auth.user_int_id())
        .await
}

pub async fn delete_tag(tag_repo: &dyn TagRepository, id: &str, auth: &AuthUser) -> AppResult<()> {
    let tag = tag_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("tag"))?;
    tag_repo.delete(tag.id, auth.tenant_id()).await?;
    Ok(())
}

pub async fn update_tag(
    tag_repo: &dyn TagRepository,
    id: &str,
    auth: &AuthUser,
    name: String,
) -> AppResult<crate::models::tag::Tag> {
    let slug = slugify(&name);
    let tag = tag_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("tag"))?;
    tag_repo
        .update(tag.id, &name, &slug, auth.tenant_id())
        .await
}

pub async fn list_tags(
    tag_repo: &dyn TagRepository,
    auth: &AuthUser,
) -> AppResult<Vec<crate::models::tag::Tag>> {
    tag_repo.find_all(auth.tenant_id()).await
}

pub async fn list_tags_paginated(
    tag_repo: &dyn TagRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<crate::models::tag::Tag>, i64)> {
    tag_repo
        .find_paginated(auth.tenant_id(), page, page_size)
        .await
}

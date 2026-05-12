//! 标签服务。

use slug::slugify;

use crate::dto::CreateTagRequest;
use crate::errors::app_error::{AppError, AppResult};
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

pub async fn get_tag(
    tag_repo: &dyn TagRepository,
    id: &str,
    auth: &AuthUser,
) -> AppResult<crate::models::tag::Tag> {
    tag_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("tag"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::CreateTagRequest;
    use crate::repositories::sqlx_tag::SqlxTagRepository;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn auth() -> AuthUser {
        AuthUser::from_parts(
            Some("u1".to_string()),
            Some(1),
            crate::models::user::UserRole::Admin,
            None,
        )
    }

    #[tokio::test]
    async fn create_tag_basic() {
        let pool = setup_pool().await;
        let repo = SqlxTagRepository::new(pool.clone());
        let a = auth();
        let tag = super::create_tag(
            &repo,
            &a,
            CreateTagRequest {
                name: "Rust".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(tag.name, "Rust");
        assert_eq!(tag.slug, "rust");
    }

    #[tokio::test]
    async fn list_tags_empty() {
        let pool = setup_pool().await;
        let repo = SqlxTagRepository::new(pool.clone());
        let a = auth();
        let tags = super::list_tags(&repo, &a).await.unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn update_tag() {
        let pool = setup_pool().await;
        let repo = SqlxTagRepository::new(pool.clone());
        let a = auth();
        let tag = super::create_tag(&repo, &a, CreateTagRequest { name: "Old".into() })
            .await
            .unwrap();
        let updated = super::update_tag(&repo, &tag.document_id, &a, "New".into())
            .await
            .unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.slug, "new");
    }

    #[tokio::test]
    async fn delete_tag() {
        let pool = setup_pool().await;
        let repo = SqlxTagRepository::new(pool.clone());
        let a = auth();
        let tag = super::create_tag(&repo, &a, CreateTagRequest { name: "Del".into() })
            .await
            .unwrap();
        super::delete_tag(&repo, &tag.document_id, &a)
            .await
            .unwrap();
        let tags = super::list_tags(&repo, &a).await.unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn delete_tag_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxTagRepository::new(pool.clone());
        let a = auth();
        assert!(super::delete_tag(&repo, "no-such-tag", &a).await.is_err());
    }

    #[tokio::test]
    async fn update_tag_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxTagRepository::new(pool.clone());
        let a = auth();
        assert!(
            super::update_tag(&repo, "missing", &a, "X".into())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_tags_paginated() {
        let pool = setup_pool().await;
        let repo = SqlxTagRepository::new(pool.clone());
        let a = auth();
        for i in 0..5 {
            super::create_tag(
                &repo,
                &a,
                CreateTagRequest {
                    name: format!("Tag{i}"),
                },
            )
            .await
            .unwrap();
        }
        let (tags, total) = super::list_tags_paginated(&repo, &a, 1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(tags.len(), 3);
    }
}

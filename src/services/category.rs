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

pub async fn get_category(
    category_repo: &dyn CategoryRepository,
    id: &str,
    auth: &AuthUser,
) -> AppResult<crate::models::category::Category> {
    category_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("category"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::CreateCategoryRequest;
    use crate::repositories::sqlx_category::SqlxCategoryRepository;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn auth(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some("u1".to_string()),
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string()),
        )
    }

    #[tokio::test]
    async fn create_category_basic() {
        let pool = setup_pool().await;
        let repo = SqlxCategoryRepository::new(pool.clone());
        let a = auth(None);
        let cat = super::create_category(
            &repo,
            &a,
            CreateCategoryRequest {
                name: "Tech".into(),
                description: Some("Technology".into()),
                parent_id: None,
                sort_order: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(cat.name, "Tech");
        assert_eq!(cat.slug, "tech");
    }

    #[tokio::test]
    async fn list_categories_empty() {
        let pool = setup_pool().await;
        let repo = SqlxCategoryRepository::new(pool.clone());
        let a = auth(None);
        let cats = super::list_categories(&repo, &a).await.unwrap();
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn update_category_renames() {
        let pool = setup_pool().await;
        let repo = SqlxCategoryRepository::new(pool.clone());
        let a = auth(None);
        let cat = super::create_category(
            &repo,
            &a,
            CreateCategoryRequest {
                name: "Old".into(),
                description: None,
                parent_id: None,
                sort_order: None,
            },
        )
        .await
        .unwrap();
        let updated = super::update_category(
            &repo,
            &a,
            &cat.document_id,
            crate::dto::UpdateCategoryRequest {
                name: Some("New".into()),
                description: None,
                parent_id: None,
                sort_order: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.slug, "new");
    }

    #[tokio::test]
    async fn delete_category() {
        let pool = setup_pool().await;
        let repo = SqlxCategoryRepository::new(pool.clone());
        let a = auth(None);
        let cat = super::create_category(
            &repo,
            &a,
            CreateCategoryRequest {
                name: "Del".into(),
                description: None,
                parent_id: None,
                sort_order: None,
            },
        )
        .await
        .unwrap();
        super::delete_category(&repo, &cat.document_id, &a)
            .await
            .unwrap();
        let cats = super::list_categories(&repo, &a).await.unwrap();
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn delete_category_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxCategoryRepository::new(pool.clone());
        let a = auth(None);
        assert!(
            super::delete_category(&repo, "nonexistent", &a)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_categories_paginated() {
        let pool = setup_pool().await;
        let repo = SqlxCategoryRepository::new(pool.clone());
        let a = auth(None);
        for i in 0..5 {
            super::create_category(
                &repo,
                &a,
                CreateCategoryRequest {
                    name: format!("Cat{i}"),
                    description: None,
                    parent_id: None,
                    sort_order: None,
                },
            )
            .await
            .unwrap();
        }
        let (cats, total) = super::list_categories_paginated(&repo, &a, 1, 3)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(cats.len(), 3);
    }
}

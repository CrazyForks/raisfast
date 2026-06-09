//! Category service.

use std::sync::Arc;

use async_trait::async_trait;
use raisfast_derive::aspect_service;

use crate::aspects::engine::AspectEngine;
use crate::aspects::slug_aspect;
use crate::commands::{CreateCategoryCmd, UpdateCategoryCmd};
use crate::dto::{CreateCategoryRequest, UpdateCategoryRequest};
use crate::errors::app_error::AppResult;
use crate::middleware::auth::AuthUser;
use crate::models::category::Category;
use crate::types::snowflake_id::SnowflakeId;

/// Category business logic trait.
#[async_trait]
pub trait CategoryService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreateCategoryRequest) -> AppResult<Category>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateCategoryRequest,
    ) -> AppResult<Category>;
    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()>;
    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Category>;
    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Category>>;
    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)>;
}

#[aspect_service(entity = "categories", model = Category)]
pub struct CategoryServiceImpl {
    #[engine]
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
}

#[async_trait]
impl CategoryService for CategoryServiceImpl {
    async fn create(&self, auth: &AuthUser, req: CreateCategoryRequest) -> AppResult<Category> {
        let (req, _d) = self.before_create(auth, req).await?;
        let slug = slug_aspect::generate_slug(&req.name);
        let parent_id = if let Some(ref raw_id) = req.parent_id {
            if raw_id.parse::<i64>().is_ok() {
                raw_id.parse::<i64>().ok()
            } else {
                let pid = crate::types::snowflake_id::parse_id(raw_id)?;
                let parent =
                    crate::models::category::find_by_id(&self.pool, pid, auth.tenant_id()).await?;
                Some(*parent.id)
            }
        } else {
            None
        };
        let cmd = CreateCategoryCmd {
            name: req.name,
            slug,
            description: req.description,
            parent_id,
            sort_order: req.sort_order.unwrap_or(0),
            cover_image: req.cover_image,
            meta_title: req.meta_title,
            meta_description: req.meta_description,
            og_title: req.og_title,
            og_description: req.og_description,
            og_image: req.og_image,
        };
        let cat =
            crate::models::category::create(&self.pool, &cmd, auth.tenant_id(), auth.user_id())
                .await?;
        self.after_created(&cat);
        Ok(cat)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateCategoryRequest,
    ) -> AppResult<Category> {
        let existing =
            crate::models::category::find_by_id(&self.pool, id, auth.tenant_id()).await?;
        let (req, _d) = self.before_update(auth, &existing, req).await?;
        let new_slug = if let Some(ref slug_val) = req.slug {
            if slug_val.is_empty() {
                slug_aspect::generate_slug(existing.name.as_str())
            } else {
                slug_aspect::generate_slug(slug_val)
            }
        } else if let Some(ref n) = req.name {
            slug_aspect::generate_slug(n)
        } else {
            existing.slug
        };

        let parent_id = if let Some(ref raw_id) = req.parent_id {
            if raw_id.parse::<i64>().is_ok() {
                raw_id.parse::<i64>().ok()
            } else {
                let pid = crate::types::snowflake_id::parse_id(raw_id)?;
                let parent =
                    crate::models::category::find_by_id(&self.pool, pid, auth.tenant_id()).await?;
                Some(*parent.id)
            }
        } else {
            None
        };
        let cmd = UpdateCategoryCmd {
            id: existing.id,
            name: req.name,
            slug: Some(new_slug),
            description: req.description,
            parent_id,
            sort_order: req.sort_order,
            cover_image: req.cover_image,
            meta_title: req.meta_title,
            meta_description: req.meta_description,
            og_title: req.og_title,
            og_description: req.og_description,
            og_image: req.og_image,
        };
        let updated =
            crate::models::category::update(&self.pool, &cmd, auth.tenant_id(), auth.user_id())
                .await?;
        self.after_updated(&updated);
        Ok(updated)
    }

    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()> {
        let existing =
            crate::models::category::find_by_id(&self.pool, id, auth.tenant_id()).await?;
        self.before_delete(auth, &existing).await?;
        crate::models::category::ensure_safe_to_delete(&self.pool, existing.id, auth.tenant_id())
            .await?;
        crate::models::category::delete(&self.pool, existing.id, auth.tenant_id()).await?;
        self.after_deleted(&existing);
        Ok(())
    }

    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Category> {
        crate::models::category::find_by_id(&self.pool, id, auth.tenant_id()).await
    }

    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Category>> {
        crate::models::category::find_all(&self.pool, auth.tenant_id()).await
    }

    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)> {
        crate::models::category::find_paginated(&self.pool, auth.tenant_id(), page, page_size).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::CreateCategoryRequest;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn auth(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string()),
        )
    }

    fn make_service(pool: crate::db::Pool) -> Arc<dyn CategoryService> {
        Arc::new(CategoryServiceImpl::new(
            Arc::new(AspectEngine::new()),
            Arc::new(pool),
        ))
    }

    #[tokio::test]
    async fn create_category_basic() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
                &a,
                CreateCategoryRequest {
                    name: "Tech".into(),
                    slug: None,
                    description: Some("Technology".into()),
                    parent_id: None,
                    sort_order: None,
                    cover_image: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
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
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cats = svc.list(&a).await.unwrap();
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn update_category_renames() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
                &a,
                CreateCategoryRequest {
                    name: "Old".into(),
                    slug: None,
                    description: None,
                    parent_id: None,
                    sort_order: None,
                    cover_image: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
                &a,
                cat.id,
                crate::dto::UpdateCategoryRequest {
                    name: Some("New".into()),
                    slug: None,
                    description: None,
                    parent_id: None,
                    sort_order: None,
                    cover_image: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
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
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
                &a,
                CreateCategoryRequest {
                    name: "Del".into(),
                    slug: None,
                    description: None,
                    parent_id: None,
                    sort_order: None,
                    cover_image: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                },
            )
            .await
            .unwrap();
        svc.delete(cat.id, &a).await.unwrap();
        let cats = svc.list(&a).await.unwrap();
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn delete_category_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        assert!(svc.delete(SnowflakeId(999999), &a).await.is_err());
    }

    #[tokio::test]
    async fn list_categories_paginated() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        for i in 0..5 {
            svc.create(
                &a,
                CreateCategoryRequest {
                    name: format!("Cat{i}"),
                    slug: None,
                    description: None,
                    parent_id: None,
                    sort_order: None,
                    cover_image: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                },
            )
            .await
            .unwrap();
        }
        let (cats, total) = svc.list_paginated(&a, 1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(cats.len(), 3);
    }
}

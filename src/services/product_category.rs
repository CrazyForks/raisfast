use std::sync::Arc;

use async_trait::async_trait;
use raisfast_derive::aspect_service;

use crate::aspects::engine::AspectEngine;
use crate::aspects::slug_aspect;
use crate::commands::{CreateProductCategoryCmd, UpdateProductCategoryCmd};
use crate::dto::{CreateProductCategoryRequest, UpdateProductCategoryRequest};
use crate::errors::app_error::AppResult;
use crate::middleware::auth::AuthUser;
use crate::models::product_category::ProductCategory;
use crate::types::snowflake_id::SnowflakeId;

#[async_trait]
pub trait ProductCategoryService: Send + Sync {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateProductCategoryRequest,
    ) -> AppResult<ProductCategory>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductCategoryRequest,
    ) -> AppResult<ProductCategory>;
    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()>;
    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<ProductCategory>;
    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<ProductCategory>>;
    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<ProductCategory>, i64)>;
}

#[aspect_service(entity = "product_categories", model = ProductCategory)]
pub struct ProductCategoryServiceImpl {
    #[engine]
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
}

#[async_trait]
impl ProductCategoryService for ProductCategoryServiceImpl {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateProductCategoryRequest,
    ) -> AppResult<ProductCategory> {
        let (req, _d) = self.before_create(auth, req).await?;
        let slug = slug_aspect::generate_slug(&req.name);
        let parent_id = resolve_parent_id(&self.pool, auth, req.parent_id.as_deref()).await?;
        let cmd = CreateProductCategoryCmd {
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
        let cat = crate::models::product_category::create(
            &self.pool,
            &cmd,
            auth.tenant_id(),
            auth.user_id(),
        )
        .await?;
        self.after_created(&cat);
        Ok(cat)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductCategoryRequest,
    ) -> AppResult<ProductCategory> {
        let existing =
            crate::models::product_category::find_by_id(&self.pool, id, auth.tenant_id()).await?;
        let (req, _d) = self.before_update(auth, &existing, req).await?;
        let new_slug = req
            .name
            .as_ref()
            .map(|n| slug_aspect::generate_slug(n))
            .unwrap_or(existing.slug);
        let parent_id = resolve_parent_id(&self.pool, auth, req.parent_id.as_deref()).await?;
        let cmd = UpdateProductCategoryCmd {
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
        let updated = crate::models::product_category::update(
            &self.pool,
            &cmd,
            auth.tenant_id(),
            auth.user_id(),
        )
        .await?;
        self.after_updated(&updated);
        Ok(updated)
    }

    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()> {
        let existing =
            crate::models::product_category::find_by_id(&self.pool, id, auth.tenant_id()).await?;
        self.before_delete(auth, &existing).await?;
        crate::models::product_category::ensure_safe_to_delete(
            &self.pool,
            existing.id,
            auth.tenant_id(),
        )
        .await?;
        crate::models::product_category::delete(&self.pool, existing.id, auth.tenant_id()).await?;
        self.after_deleted(&existing);
        Ok(())
    }

    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<ProductCategory> {
        crate::models::product_category::find_by_id(&self.pool, id, auth.tenant_id()).await
    }

    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<ProductCategory>> {
        crate::models::product_category::find_all(&self.pool, auth.tenant_id()).await
    }

    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<ProductCategory>, i64)> {
        crate::models::product_category::find_paginated(
            &self.pool,
            auth.tenant_id(),
            page,
            page_size,
        )
        .await
    }
}

async fn resolve_parent_id(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    raw_id: Option<&str>,
) -> AppResult<Option<i64>> {
    match raw_id {
        Some(raw) if !raw.is_empty() => {
            let pid = crate::types::snowflake_id::parse_id(raw)?;
            let parent =
                crate::models::product_category::find_by_id(pool, pid, auth.tenant_id()).await?;
            Ok(Some(*parent.id))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{CreateProductCategoryRequest, UpdateProductCategoryRequest};

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn auth(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string())
                .or_else(|| Some("default".to_string())),
        )
    }

    fn make_service(pool: crate::db::Pool) -> Arc<dyn ProductCategoryService> {
        Arc::new(ProductCategoryServiceImpl::new(
            Arc::new(AspectEngine::new()),
            Arc::new(pool),
        ))
    }

    #[tokio::test]
    async fn create_basic() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
                &a,
                CreateProductCategoryRequest {
                    name: "Electronics".into(),
                    description: Some("All electronic items".into()),
                    parent_id: None,
                    sort_order: Some(0),
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
        assert_eq!(cat.name, "Electronics");
        assert_eq!(cat.slug, "electronics");
        assert_eq!(cat.description.unwrap(), "All electronic items");
    }

    #[tokio::test]
    async fn get_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
                &a,
                CreateProductCategoryRequest {
                    name: "Phones".into(),
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
        let found = svc.get(cat.id, &a).await.unwrap();
        assert_eq!(found.id, cat.id);
        assert_eq!(found.name, "Phones");
    }

    #[tokio::test]
    async fn get_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        assert!(svc.get(SnowflakeId(0), &a).await.is_err());
    }

    #[tokio::test]
    async fn update_changes_name() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
                &a,
                CreateProductCategoryRequest {
                    name: "Old".into(),
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
                UpdateProductCategoryRequest {
                    name: Some("New".into()),
                    description: Some("updated".into()),
                    parent_id: None,
                    sort_order: Some(5),
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
        assert_eq!(updated.description.unwrap(), "updated");
        assert_eq!(updated.sort_order, 5);
    }

    #[tokio::test]
    async fn delete_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
                &a,
                CreateProductCategoryRequest {
                    name: "Bye".into(),
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
        assert!(svc.get(cat.id, &a).await.is_err());
    }

    #[tokio::test]
    async fn delete_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        assert!(svc.delete(SnowflakeId(0), &a).await.is_err());
    }

    #[tokio::test]
    async fn list_returns_all() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        svc.create(
            &a,
            CreateProductCategoryRequest {
                name: "A".into(),
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
        svc.create(
            &a,
            CreateProductCategoryRequest {
                name: "B".into(),
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
        let all = svc.list(&a).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_paginated() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        for name in ["A", "B", "C", "D", "E"] {
            svc.create(
                &a,
                CreateProductCategoryRequest {
                    name: name.to_string(),
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
        let (items, total) = svc.list_paginated(&a, 1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
        let (items2, total2) = svc.list_paginated(&a, 2, 3).await.unwrap();
        assert_eq!(total2, 5);
        assert_eq!(items2.len(), 2);
    }

    #[tokio::test]
    async fn create_with_parent() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let parent = svc
            .create(
                &a,
                CreateProductCategoryRequest {
                    name: "Parent".into(),
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
        let child = svc
            .create(
                &a,
                CreateProductCategoryRequest {
                    name: "Child".into(),
                    description: None,
                    parent_id: Some(parent.id.to_string()),
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
        assert_eq!(child.parent_id.unwrap(), parent.id);
    }

    #[tokio::test]
    async fn create_with_invalid_parent_returns_error() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let result = svc
            .create(
                &a,
                CreateProductCategoryRequest {
                    name: "Orphan".into(),
                    description: None,
                    parent_id: Some("99999".to_string()),
                    sort_order: None,
                    cover_image: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                },
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a1 = auth(Some("t1"));
        let a2 = auth(Some("t2"));
        svc.create(
            &a1,
            CreateProductCategoryRequest {
                name: "T1Cat".into(),
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
        svc.create(
            &a2,
            CreateProductCategoryRequest {
                name: "T2Cat".into(),
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
        let t1_items = svc.list(&a1).await.unwrap();
        let t2_items = svc.list(&a2).await.unwrap();
        assert_eq!(t1_items.len(), 1);
        assert_eq!(t1_items[0].name, "T1Cat");
        assert_eq!(t2_items.len(), 1);
        assert_eq!(t2_items[0].name, "T2Cat");
    }
}

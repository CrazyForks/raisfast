//! Category service.

use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::aspects::slug_aspect;
use crate::commands::{CreateCategoryCmd, UpdateCategoryCmd};
use crate::dto::{CreateCategoryRequest, UpdateCategoryRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::category::Category;
use crate::repositories::CategoryRepository;

/// Category business logic trait.
#[async_trait]
pub trait CategoryService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreateCategoryRequest) -> AppResult<Category>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: &str,
        req: UpdateCategoryRequest,
    ) -> AppResult<Category>;
    async fn delete(&self, id: &str, auth: &AuthUser) -> AppResult<()>;
    async fn get(&self, id: &str, auth: &AuthUser) -> AppResult<Category>;
    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Category>>;
    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)>;
}

pub struct CategoryServiceImpl {
    repo: Arc<dyn CategoryRepository>,
    aspect_engine: Arc<AspectEngine>,
}

impl CategoryServiceImpl {
    pub fn new(repo: Arc<dyn CategoryRepository>, aspect_engine: Arc<AspectEngine>) -> Self {
        Self {
            repo,
            aspect_engine,
        }
    }

    async fn before_create(
        &self,
        auth: &AuthUser,
        req: CreateCategoryRequest,
    ) -> AppResult<(CreateCategoryRequest, crate::aspects::Dispatched)> {
        self.aspect_engine
            .before_create("categories", auth, req)
            .await
    }

    async fn before_update(
        &self,
        auth: &AuthUser,
        existing: &Category,
        req: UpdateCategoryRequest,
    ) -> AppResult<(UpdateCategoryRequest, crate::aspects::Dispatched)> {
        self.aspect_engine
            .before_update("categories", auth, existing, req)
            .await
    }

    async fn before_delete(
        &self,
        auth: &AuthUser,
        existing: &Category,
    ) -> AppResult<crate::aspects::Dispatched> {
        self.aspect_engine
            .before_delete("categories", auth, existing)
            .await
    }

    fn after_created(&self, cat: &Category) {
        self.aspect_engine.emit(Event::CategoryCreated(cat.clone()));
    }

    fn after_updated(&self, cat: &Category) {
        self.aspect_engine.emit(Event::CategoryUpdated(cat.clone()));
    }

    fn after_deleted(&self, cat: &Category) {
        self.aspect_engine.emit(Event::CategoryDeleted(cat.clone()));
    }
}

#[async_trait]
impl CategoryService for CategoryServiceImpl {
    async fn create(&self, auth: &AuthUser, req: CreateCategoryRequest) -> AppResult<Category> {
        let (req, _d) = self.before_create(auth, req).await?;
        let slug = slug_aspect::generate_slug(&req.name);
        let parent_id = if let Some(ref doc_id) = req.parent_id {
            if doc_id.parse::<i64>().is_ok() {
                doc_id.parse::<i64>().ok()
            } else {
                self.repo
                    .find_by_document_id(doc_id, auth.tenant_id())
                    .await?
                    .map(|c| c.id)
            }
        } else {
            None
        };
        let cat = self
            .repo
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
            .await?;
        self.after_created(&cat);
        Ok(cat)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: &str,
        req: UpdateCategoryRequest,
    ) -> AppResult<Category> {
        let existing = self
            .repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("category"))?;
        let (req, _d) = self.before_update(auth, &existing, req).await?;
        let new_slug = req
            .name
            .as_ref()
            .map(|n| slug_aspect::generate_slug(n))
            .unwrap_or(existing.slug);

        let parent_id = if let Some(ref doc_id) = req.parent_id {
            if doc_id.parse::<i64>().is_ok() {
                doc_id.parse::<i64>().ok()
            } else {
                self.repo
                    .find_by_document_id(doc_id, auth.tenant_id())
                    .await?
                    .map(|c| c.id)
            }
        } else {
            None
        };
        let updated = self
            .repo
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
            .await?;
        self.after_updated(&updated);
        Ok(updated)
    }

    async fn delete(&self, id: &str, auth: &AuthUser) -> AppResult<()> {
        let existing = self
            .repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("category"))?;
        self.before_delete(auth, &existing).await?;
        self.repo.delete(existing.id, auth.tenant_id()).await?;
        self.after_deleted(&existing);
        Ok(())
    }

    async fn get(&self, id: &str, auth: &AuthUser) -> AppResult<Category> {
        self.repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("category"))
    }

    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Category>> {
        self.repo.find_all(auth.tenant_id()).await
    }

    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)> {
        self.repo
            .find_paginated(auth.tenant_id(), page, page_size)
            .await
    }
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

    fn make_service(pool: crate::db::Pool) -> Arc<dyn CategoryService> {
        Arc::new(CategoryServiceImpl::new(
            Arc::new(SqlxCategoryRepository::new(pool.clone())),
            Arc::new(AspectEngine::new()),
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
                    description: None,
                    parent_id: None,
                    sort_order: None,
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
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
        let svc = make_service(pool.clone());
        let a = auth(None);
        let cat = svc
            .create(
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
        svc.delete(&cat.document_id, &a).await.unwrap();
        let cats = svc.list(&a).await.unwrap();
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn delete_category_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        assert!(svc.delete("nonexistent", &a).await.is_err());
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
                    description: None,
                    parent_id: None,
                    sort_order: None,
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

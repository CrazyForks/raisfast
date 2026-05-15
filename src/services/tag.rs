//! Tag service.

use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::dto::CreateTagRequest;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::tag::Tag;
use crate::repositories::TagRepository;

pub fn generate_slug(name: &str) -> String {
    crate::aspects::slug_aspect::generate_slug(name)
}

#[async_trait]
pub trait TagService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreateTagRequest) -> AppResult<Tag>;
    async fn update(&self, auth: &AuthUser, id: &str, name: String, slug: String)
    -> AppResult<Tag>;
    async fn delete(&self, id: &str, auth: &AuthUser) -> AppResult<()>;
    async fn get(&self, id: &str, auth: &AuthUser) -> AppResult<Tag>;
    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Tag>>;
    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)>;
}

pub struct TagServiceImpl {
    repo: Arc<dyn TagRepository>,
    aspect_engine: Arc<AspectEngine>,
}

impl TagServiceImpl {
    pub fn new(repo: Arc<dyn TagRepository>, aspect_engine: Arc<AspectEngine>) -> Self {
        Self {
            repo,
            aspect_engine,
        }
    }

    async fn before_create(
        &self,
        auth: &AuthUser,
        req: CreateTagRequest,
    ) -> AppResult<(CreateTagRequest, crate::aspects::Dispatched)> {
        self.aspect_engine.before_create("tags", auth, req).await
    }

    async fn before_update(
        &self,
        auth: &AuthUser,
        existing: &Tag,
        req: (String, String),
    ) -> AppResult<((String, String), crate::aspects::Dispatched)> {
        self.aspect_engine
            .before_update("tags", auth, existing, req)
            .await
    }

    async fn before_delete(
        &self,
        auth: &AuthUser,
        existing: &Tag,
    ) -> AppResult<crate::aspects::Dispatched> {
        self.aspect_engine
            .before_delete("tags", auth, existing)
            .await
    }

    fn after_created(&self, tag: &Tag) {
        self.aspect_engine.emit(Event::TagCreated(tag.clone()));
    }

    fn after_updated(&self, tag: &Tag) {
        self.aspect_engine.emit(Event::TagUpdated(tag.clone()));
    }

    fn after_deleted(&self, tag: &Tag) {
        self.aspect_engine.emit(Event::TagDeleted(tag.clone()));
    }
}

#[async_trait]
impl TagService for TagServiceImpl {
    async fn create(&self, auth: &AuthUser, req: CreateTagRequest) -> AppResult<Tag> {
        let (req, _d) = self.before_create(auth, req).await?;
        let slug = generate_slug(&req.name);
        let tag = self
            .repo
            .create(&req.name, &slug, auth.tenant_id(), auth.user_int_id())
            .await?;
        self.after_created(&tag);
        Ok(tag)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: &str,
        name: String,
        slug: String,
    ) -> AppResult<Tag> {
        let tag = self
            .repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("tag"))?;
        let ((name, slug), _d) = self.before_update(auth, &tag, (name, slug)).await?;
        let updated = self
            .repo
            .update(tag.id, &name, &slug, auth.tenant_id())
            .await?;
        self.after_updated(&updated);
        Ok(updated)
    }

    async fn delete(&self, id: &str, auth: &AuthUser) -> AppResult<()> {
        let tag = self
            .repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("tag"))?;
        self.before_delete(auth, &tag).await?;
        self.repo.delete(tag.id, auth.tenant_id()).await?;
        self.after_deleted(&tag);
        Ok(())
    }

    async fn get(&self, id: &str, auth: &AuthUser) -> AppResult<Tag> {
        self.repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("tag"))
    }

    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Tag>> {
        self.repo.find_all(auth.tenant_id()).await
    }

    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)> {
        self.repo
            .find_paginated(auth.tenant_id(), page, page_size)
            .await
    }
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

    fn make_service(pool: crate::db::Pool) -> Arc<dyn TagService> {
        Arc::new(TagServiceImpl::new(
            Arc::new(SqlxTagRepository::new(pool.clone())),
            Arc::new(AspectEngine::new()),
        ))
    }

    #[tokio::test]
    async fn create_tag_basic() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        let tag = svc
            .create(
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
        let svc = make_service(pool.clone());
        let a = auth();
        let tags = svc.list(&a).await.unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn update_tag() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        let tag = svc
            .create(&a, CreateTagRequest { name: "Old".into() })
            .await
            .unwrap();
        let updated = svc
            .update(&a, &tag.document_id, "New".into(), generate_slug("New"))
            .await
            .unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.slug, "new");
    }

    #[tokio::test]
    async fn delete_tag() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        let tag = svc
            .create(&a, CreateTagRequest { name: "Del".into() })
            .await
            .unwrap();
        svc.delete(&tag.document_id, &a).await.unwrap();
        let tags = svc.list(&a).await.unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn delete_tag_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        assert!(svc.delete("no-such-tag", &a).await.is_err());
    }

    #[tokio::test]
    async fn update_tag_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        assert!(
            svc.update(&a, "missing", "X".into(), generate_slug("X"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_tags_paginated() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        for i in 0..5 {
            svc.create(
                &a,
                CreateTagRequest {
                    name: format!("Tag{i}"),
                },
            )
            .await
            .unwrap();
        }
        let (tags, total) = svc.list_paginated(&a, 1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(tags.len(), 3);
    }
}

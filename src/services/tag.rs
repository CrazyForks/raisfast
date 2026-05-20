//! Tag service.

use std::sync::Arc;

use async_trait::async_trait;
use raisfast_derive::aspect_service;

use crate::aspects::engine::AspectEngine;
use crate::dto::CreateTagRequest;
use crate::errors::app_error::AppResult;
use crate::middleware::auth::AuthUser;
use crate::models::tag::Tag;
use crate::types::snowflake_id::SnowflakeId;

pub fn generate_slug(name: &str) -> String {
    crate::aspects::slug_aspect::generate_slug(name)
}

#[async_trait]
pub trait TagService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreateTagRequest) -> AppResult<Tag>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        name: String,
        slug: String,
    ) -> AppResult<Tag>;
    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()>;
    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Tag>;
    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Tag>>;
    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)>;
}

#[aspect_service(entity = "tags", model = Tag)]
pub struct TagServiceImpl {
    #[engine]
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
}

#[async_trait]
impl TagService for TagServiceImpl {
    async fn create(&self, auth: &AuthUser, req: CreateTagRequest) -> AppResult<Tag> {
        let (req, _d) = self.before_create(auth, req).await?;
        let slug = generate_slug(&req.name);
        let tag = crate::models::tag::create(
            &self.pool,
            &req.name,
            &slug,
            auth.tenant_id(),
            auth.user_id(),
        )
        .await?;
        self.after_created(&tag);
        Ok(tag)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        name: String,
        slug: String,
    ) -> AppResult<Tag> {
        let tag = crate::models::tag::find_by_id(&self.pool, id, auth.tenant_id()).await?;
        let ((name, slug), _d) = self.before_update(auth, &tag, (name, slug)).await?;
        let updated =
            crate::models::tag::update(&self.pool, tag.id, &name, &slug, auth.tenant_id()).await?;
        self.after_updated(&updated);
        Ok(updated)
    }

    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()> {
        let tag = crate::models::tag::find_by_id(&self.pool, id, auth.tenant_id()).await?;
        self.before_delete(auth, &tag).await?;
        crate::models::tag::delete(&self.pool, tag.id, auth.tenant_id()).await?;
        self.after_deleted(&tag);
        Ok(())
    }

    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Tag> {
        crate::models::tag::find_by_id(&self.pool, id, auth.tenant_id()).await
    }

    async fn list(&self, auth: &AuthUser) -> AppResult<Vec<Tag>> {
        crate::models::tag::find_all(&self.pool, auth.tenant_id()).await
    }

    async fn list_paginated(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)> {
        crate::models::tag::find_paginated(&self.pool, auth.tenant_id(), page, page_size).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::CreateTagRequest;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn auth() -> AuthUser {
        AuthUser::from_parts(Some(1), crate::models::user::UserRole::Admin, None)
    }

    fn make_service(pool: crate::db::Pool) -> Arc<dyn TagService> {
        Arc::new(TagServiceImpl::new(
            Arc::new(AspectEngine::new()),
            Arc::new(pool),
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
            .update(&a, tag.id, "New".into(), generate_slug("New"))
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
        svc.delete(tag.id, &a).await.unwrap();
        let tags = svc.list(&a).await.unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn delete_tag_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        assert!(svc.delete(SnowflakeId(999999), &a).await.is_err());
    }

    #[tokio::test]
    async fn update_tag_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth();
        assert!(
            svc.update(&a, SnowflakeId(999999), "X".into(), generate_slug("X"))
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

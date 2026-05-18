//! Tag model and database queries
//!
//! Defines data structures for tags and CRUD operations on the `tags` table.
//! Tags are associated with posts via the `posts_tags` join table in a many-to-many relationship.

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Tag {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub cover_image: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub created_by: Option<i64>,
    pub updated_by: Option<i64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<Tag>> {
    raisfast_derive::crud_list!(pool, "tags", Tag, order_by: "name", tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Tag>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, Tag,
        data_sql: "SELECT * FROM tags WHERE 1=1{tenant} ORDER BY name",
        count_sql: "SELECT COUNT(*) FROM tags WHERE 1=1{tenant}",
        binds: [],
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    raisfast_derive::crud_find_one!(pool, "tags", Tag, "id" => id, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    raisfast_derive::crud_find_one!(pool, "tags", Tag, "document_id" => document_id, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn create(
    pool: &crate::db::Pool,
    name: &str,
    slug: &str,
    tenant_id: Option<&str>,
    created_by: Option<i64>,
) -> AppResult<Tag> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    raisfast_derive::crud_insert!(
        pool,
        "tags",
        [
            "document_id" => &document_id,
            "name" => name,
            "slug" => slug,
            "created_by" => created_by,
            "updated_by" => created_by,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;

    find_by_document_id(pool, &document_id, tenant_id).await
}

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(pool, "tags", "id" => id, tenant: tenant_id)?;
    AppError::expect_affected(&result, "tag")
}

pub async fn update(
    pool: &crate::db::Pool,
    id: i64,
    name: &str,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(pool, "tags",
        bind: ["name" => name, "slug" => slug, "updated_at" => &now],
        where: "id" => id,
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "tag")?;
    find_by_id(pool, id, tenant_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    #[tokio::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let tag = create(&pool, "rust", "rust", None, None).await.unwrap();
        assert_eq!(tag.name, "rust");
        assert_eq!(tag.slug, "rust");

        let found = find_by_id(&pool, tag.id, None).await.unwrap();
        assert_eq!(found.id, tag.id);
        assert_eq!(found.name, "rust");
    }

    #[tokio::test]
    async fn find_by_document_id() {
        let pool = setup_pool().await;
        let tag = create(&pool, "rust", "rust", None, None).await.unwrap();

        let found = super::find_by_document_id(&pool, &tag.document_id, None)
            .await
            .unwrap();
        assert_eq!(found.id, tag.id);
        assert_eq!(found.document_id, tag.document_id);
    }

    #[tokio::test]
    async fn find_all_returns_all() {
        let pool = setup_pool().await;
        create(&pool, "rust", "rust", None, None).await.unwrap();
        create(&pool, "axum", "axum", None, None).await.unwrap();
        create(&pool, "tokio", "tokio", None, None).await.unwrap();

        let tags = find_all(&pool, None).await.unwrap();
        assert_eq!(tags.len(), 3);
    }

    #[tokio::test]
    async fn find_paginated() {
        let pool = setup_pool().await;
        create(&pool, "rust", "rust", None, None).await.unwrap();
        create(&pool, "axum", "axum", None, None).await.unwrap();
        create(&pool, "tokio", "tokio", None, None).await.unwrap();
        create(&pool, "serde", "serde", None, None).await.unwrap();
        create(&pool, "clap", "clap", None, None).await.unwrap();

        let (items, total) = super::find_paginated(&pool, None, 1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn update_changes_name() {
        let pool = setup_pool().await;
        let tag = create(&pool, "rust", "rust", None, None).await.unwrap();

        let updated = update(&pool, tag.id, "Rust Lang", "rust-lang", None)
            .await
            .unwrap();
        assert_eq!(updated.name, "Rust Lang");
        assert_eq!(updated.slug, "rust-lang");
        assert_eq!(updated.id, tag.id);
    }

    #[tokio::test]
    async fn delete_removes_tag() {
        let pool = setup_pool().await;
        let tag = create(&pool, "rust", "rust", None, None).await.unwrap();

        delete(&pool, tag.id, None).await.unwrap();
        let result = find_by_id(&pool, tag.id, None).await;
        assert!(result.is_err());
    }
}

//! 标签模型与数据库查询
//!
//! 定义标签（Tag）的数据结构以及对 `tags` 表的增删改查操作。
//! 标签通过 `posts_tags` 关联表与文章建立多对多关系。

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::{AppError, AppResult};

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tag {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub updated_by: Option<i64>,
    pub updated_at: Option<String>,
    pub created_at: String,
}

crate::impl_from_row_opt_tenant!(Tag {
    required { id, document_id, name, slug, created_at }
    optional { updated_by, updated_at }
});

pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<Tag>> {
    let sql = format!(
        "SELECT * FROM tags WHERE 1=1{} ORDER BY name",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut q = sqlx::query_as::<_, Tag>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let tags = q.fetch_all(pool).await?;
    Ok(tags)
}

pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Tag>, i64)> {
    let offset = (page - 1).max(0) * page_size;

    let count_sql = format!(
        "SELECT COUNT(*) FROM tags WHERE 1=1{}",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let base = usize::from(tenant_id.is_some()) + 1;
    let data_sql = format!(
        "SELECT * FROM tags WHERE 1=1{} ORDER BY name LIMIT {} OFFSET {}",
        tenant_filter_ph(tenant_id, 1),
        ph(base),
        ph(base + 1)
    );
    let mut dq = sqlx::query_as::<_, Tag>(&data_sql);
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    dq = dq.bind(page_size).bind(offset);
    let items = dq.fetch_all(pool).await?;

    Ok((items, total))
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    let sql = format!(
        "SELECT * FROM tags WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Tag>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    let sql = format!(
        "SELECT * FROM tags WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Tag>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

pub async fn create(
    pool: &crate::db::Pool,
    name: &str,
    slug: &str,
    tenant_id: Option<&str>,
    created_by: Option<i64>,
) -> AppResult<Tag> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO tags (document_id, tenant_id, name, slug, created_by, updated_by, created_at) VALUES ({}, {}, {}, {}, {}, {}, {})",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(name)
                .bind(slug)
                .bind(created_by)
                .bind(created_by)
                .bind(&now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO tags (document_id, name, slug, created_by, updated_by, created_at) VALUES ({}, {}, {}, {}, {}, {})",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(name)
                .bind(slug)
                .bind(created_by)
                .bind(created_by)
                .bind(&now)
                .execute(pool)
                .await?;
        }
    }

    find_by_document_id(pool, &document_id, tenant_id).await
}

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM tags WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "tag")
}

pub async fn update(
    pool: &crate::db::Pool,
    id: i64,
    name: &str,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    let now = chrono::Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE tags SET name = {}, slug = {}, updated_at = {} WHERE id = {}{}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        tenant_filter_ph(tenant_id, 5)
    );
    let mut q = sqlx::query(&sql).bind(name).bind(slug).bind(&now).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "tag")?;
    find_by_id(pool, id, tenant_id).await
}

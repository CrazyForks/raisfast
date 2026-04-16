//! 标签模型与数据库查询
//!
//! 定义标签（Tag）的数据结构以及对 `tags` 表的增删改查操作。
//! 标签通过 `posts_tags` 关联表与文章建立多对多关系。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::tenant::{resolve_tenant, tenant_filter};
use crate::errors::app_error::{AppError, AppResult};

/// 标签完整数据库行模型
///
/// 直接映射 `tags` 表的所有字段。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Tag {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub slug: String,
    pub created_at: String,
}

/// 查询所有标签
///
/// 按 `name` 字母顺序排列返回完整标签列表。
pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<Tag>> {
    let sql_str = format!(
        "SELECT * FROM tags WHERE 1=1{} ORDER BY name",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Tag>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let tags = q.fetch_all(pool).await?;
    Ok(tags)
}

/// 分页查询标签
///
/// 返回 (当前页数据, 总条数)。
pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Tag>, i64)> {
    let offset = (page - 1).max(0) * page_size;

    let count_sql = format!(
        "SELECT COUNT(*) FROM tags WHERE 1=1{}",
        tenant_filter(tenant_id)
    );
    let count_sql = crate::db::dialect::translate(&count_sql);
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let data_sql = format!(
        "SELECT * FROM tags WHERE 1=1{} ORDER BY name LIMIT ? OFFSET ?",
        tenant_filter(tenant_id)
    );
    let data_sql = crate::db::dialect::translate(&data_sql);
    let mut dq = sqlx::query_as::<_, Tag>(&data_sql);
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    dq = dq.bind(page_size).bind(offset);
    let items = dq.fetch_all(pool).await?;

    Ok((items, total))
}

/// 根据标签 ID 查找标签
///
/// 若未找到则返回 [`AppError::NotFound`]。
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    let sql_str = format!(
        "SELECT * FROM tags WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Tag>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

/// 创建新标签
///
/// 自动生成 UUID v7 作为主键。创建完成后重新查询并返回完整标签记录。
pub async fn create(
    pool: &crate::db::Pool,
    name: &str,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Tag> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();
    let tid = resolve_tenant(tenant_id);

    let sql = crate::db::dialect::translate(
        "INSERT INTO tags (id, tenant_id, name, slug, created_at) VALUES (?, ?, ?, ?, ?)",
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(tid)
        .bind(name)
        .bind(slug)
        .bind(&now)
        .execute(pool)
        .await?;

    find_by_id(pool, &id, Some(tid)).await
}

/// 删除标签
///
/// 若标签不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!("DELETE FROM tags WHERE id = ?{}", tenant_filter(tenant_id));
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "tag")
}

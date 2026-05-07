//! 分类模型与数据库查询
//!
//! 定义内容分类的数据结构以及对 `categories` 表的增删改查操作。
//! 分类支持嵌套（通过 `parent_id`）和自定义排序（通过 `sort_order`）。

use serde::{Deserialize, Serialize};

#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::tenant::tenant_filter;
use crate::errors::app_error::{AppError, AppResult};

/// 分类完整数据库行模型
///
/// 直接映射 `categories` 表的所有字段。
/// `parent_id` 用于构建层级分类，`sort_order` 控制显示顺序。
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Category {
    pub id: String,
    pub tenant_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub updated_by: Option<String>,
    pub updated_at: Option<String>,
    pub created_at: String,
}

crate::impl_from_row_opt_tenant!(Category {
    required { id, name, slug, sort_order, created_at }
    optional { description, parent_id, updated_by, updated_at }
});

/// 查询所有分类
///
/// 按 `sort_order` 和 `name` 排序返回完整分类列表。
pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<Category>> {
    let sql_str = format!(
        "SELECT * FROM categories WHERE 1=1{} ORDER BY sort_order, name",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Category>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let categories = q.fetch_all(pool).await?;
    Ok(categories)
}

/// 分页查询分类
///
/// 返回 (当前页数据, 总条数)。
pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Category>, i64)> {
    let offset = (page - 1).max(0) * page_size;

    let count_sql = format!(
        "SELECT COUNT(*) FROM categories WHERE 1=1{}",
        tenant_filter(tenant_id)
    );
    let count_sql = crate::db::dialect::translate(&count_sql);
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let data_sql = format!(
        "SELECT * FROM categories WHERE 1=1{} ORDER BY sort_order, name LIMIT ? OFFSET ?",
        tenant_filter(tenant_id)
    );
    let data_sql = crate::db::dialect::translate(&data_sql);
    let mut dq = sqlx::query_as::<_, Category>(&data_sql);
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    dq = dq.bind(page_size).bind(offset);
    let items = dq.fetch_all(pool).await?;

    Ok((items, total))
}

/// 根据分类 ID 查找分类
///
/// 若未找到则返回 [`AppError::NotFound`]。
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Category> {
    let sql_str = format!(
        "SELECT * FROM categories WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Category>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

/// 创建新分类
///
/// 自动生成 UUID v7 作为主键。创建完成后重新查询并返回完整分类记录。
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateCategoryCmd,
    tenant_id: Option<&str>,
    created_by: Option<&str>,
) -> AppResult<Category> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();

    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO categories (id, tenant_id, name, slug, description, parent_id, sort_order, created_by, updated_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(tid)
                .bind(&cmd.name)
                .bind(&cmd.slug)
                .bind(&cmd.description)
                .bind(&cmd.parent_id)
                .bind(cmd.sort_order)
                .bind(created_by)
                .bind(created_by)
                .bind(&now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = crate::db::dialect::translate(
                "INSERT INTO categories (id, name, slug, description, parent_id, sort_order, created_by, updated_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(&cmd.name)
                .bind(&cmd.slug)
                .bind(&cmd.description)
                .bind(&cmd.parent_id)
                .bind(cmd.sort_order)
                .bind(created_by)
                .bind(created_by)
                .bind(&now)
                .execute(pool)
                .await?;
        }
    }

    find_by_id(pool, &id, tenant_id).await
}

/// 更新分类
///
/// 仅更新传入的非空字段，其余保留原值。
pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateCategoryCmd,
    tenant_id: Option<&str>,
    updated_by: Option<&str>,
) -> AppResult<Category> {
    let existing = find_by_id(pool, &cmd.id, tenant_id).await?;
    let now = crate::utils::tz::now_str();

    let name = cmd.name.as_deref().unwrap_or(&existing.name);
    let slug = cmd.slug.as_deref().unwrap_or(&existing.slug);
    let desc = cmd
        .description
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.description);
    let parent = cmd
        .parent_id
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.parent_id);
    let sort = cmd.sort_order.unwrap_or(existing.sort_order);

    let sql = format!(
        "UPDATE categories SET name = ?, slug = ?, description = ?, parent_id = ?, sort_order = ?, updated_by = ?, updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql)
        .bind(name)
        .bind(slug)
        .bind(desc)
        .bind(parent)
        .bind(sort)
        .bind(updated_by)
        .bind(&now)
        .bind(&cmd.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;

    find_by_id(pool, &cmd.id, tenant_id).await
}

/// 删除分类
///
/// 若分类不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM categories WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "category")
}

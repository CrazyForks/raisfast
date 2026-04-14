//! 分类模型与数据库查询
//!
//! 定义博客文章分类的数据结构以及对 `categories` 表的增删改查操作。
//! 分类支持嵌套（通过 `parent_id`）和自定义排序（通过 `sort_order`）。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::errors::app_error::{AppError, AppResult};

/// 分类完整数据库行模型
///
/// 直接映射 `categories` 表的所有字段。
/// `parent_id` 用于构建层级分类，`sort_order` 控制显示顺序。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub created_at: String,
}

/// 查询所有分类
///
/// 按 `sort_order` 和 `name` 排序返回完整分类列表。
pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<Category>> {
    let categories =
        sqlx::query_as::<_, Category>("SELECT * FROM categories ORDER BY sort_order, name")
            .fetch_all(pool)
            .await?;
    Ok(categories)
}

/// 根据分类 ID 查找分类
///
/// 若未找到则返回 [`AppError::NotFound`]。
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Category> {
    let sql = crate::db::dialect::translate("SELECT * FROM categories WHERE id = ?");
    sqlx::query_as::<_, Category>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// 创建新分类
///
/// 自动生成 UUID v7 作为主键。创建完成后重新查询并返回完整分类记录。
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateCategoryCmd,
) -> AppResult<Category> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        "INSERT INTO categories (id, name, slug, description, parent_id, sort_order, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        id,
        cmd.name,
        cmd.slug,
        cmd.description,
        cmd.parent_id,
        cmd.sort_order,
        now,
    )
    .execute(pool)
    .await?;

    find_by_id(pool, &id).await
}

/// 更新分类
///
/// 仅更新传入的非空字段，其余保留原值。
pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateCategoryCmd,
) -> AppResult<Category> {
    let existing = find_by_id(pool, &cmd.id).await?;

    let name = cmd.name.as_deref().unwrap_or(&existing.name);
    let slug = cmd.slug.as_deref().unwrap_or(&existing.slug);
    let desc = cmd
        .description
        .as_deref()
        .map(|s| s.to_string())
        .or(existing.description);
    let parent = cmd
        .parent_id
        .as_deref()
        .map(|s| s.to_string())
        .or(existing.parent_id);
    let sort = cmd.sort_order.unwrap_or(existing.sort_order);

    sqlx::query!(
        "UPDATE categories SET name = ?, slug = ?, description = ?, parent_id = ?, sort_order = ? WHERE id = ?",
        name,
        slug,
        desc,
        parent,
        sort,
        cmd.id,
    )
    .execute(pool)
    .await?;

    find_by_id(pool, &cmd.id).await
}

/// 删除分类
///
/// 若分类不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let result = sqlx::query!("DELETE FROM categories WHERE id = ?", id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("category".into()));
    }
    Ok(())
}

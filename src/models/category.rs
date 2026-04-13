//! 分类模型与数据库查询
//!
//! 定义博客文章分类的数据结构以及对 `categories` 表的增删改查操作。
//! 分类支持嵌套（通过 `parent_id`）和自定义排序（通过 `sort_order`）。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

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

/// 创建分类请求体
///
/// - `name` 长度 1–100 个字符
/// - `sort_order` 可选，默认为 0
#[derive(Debug, Deserialize, Validate)]
pub struct CreateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

/// 更新分类请求体
///
/// 所有字段均为可选，仅更新提供的字段。
/// - `name` 如果提供，长度须在 1–100 个字符之间
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateCategoryRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

/// 查询所有分类
///
/// 按 `sort_order` 和 `name` 排序返回完整分类列表。
pub async fn find_all(pool: &sqlx::SqlitePool) -> AppResult<Vec<Category>> {
    let categories =
        sqlx::query_as::<_, Category>("SELECT * FROM categories ORDER BY sort_order, name")
            .fetch_all(pool)
            .await?;
    Ok(categories)
}

/// 根据分类 ID 查找分类
///
/// 若未找到则返回 [`AppError::NotFound`]。
pub async fn find_by_id(pool: &sqlx::SqlitePool, id: &str) -> AppResult<Category> {
    sqlx::query_as::<_, Category>("SELECT * FROM categories WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// 创建新分类
///
/// 自动生成 UUID v7 作为主键。创建完成后重新查询并返回完整分类记录。
pub async fn create(
    pool: &sqlx::SqlitePool,
    name: &str,
    slug: &str,
    description: Option<&str>,
    parent_id: Option<&str>,
    sort_order: i64,
) -> AppResult<Category> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO categories (id, name, slug, description, parent_id, sort_order, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(slug)
    .bind(description)
    .bind(parent_id)
    .bind(sort_order)
    .bind(&now)
    .execute(pool)
    .await?;

    find_by_id(pool, &id).await
}

/// 更新分类
///
/// 仅更新传入的非空字段，其余保留原值。
pub async fn update(
    pool: &sqlx::SqlitePool,
    id: &str,
    name: Option<&str>,
    slug: Option<&str>,
    description: Option<&str>,
    parent_id: Option<&str>,
    sort_order: Option<i64>,
) -> AppResult<Category> {
    let existing = find_by_id(pool, id).await?;

    let name = name.unwrap_or(&existing.name);
    let slug = slug.unwrap_or(&existing.slug);
    let desc = description.map(|s| s.to_string()).or(existing.description);
    let parent = parent_id.map(|s| s.to_string()).or(existing.parent_id);
    let sort = sort_order.unwrap_or(existing.sort_order);

    sqlx::query(
        "UPDATE categories SET name = ?, slug = ?, description = ?, parent_id = ?, sort_order = ? WHERE id = ?",
    )
    .bind(name)
    .bind(slug)
    .bind(&desc)
    .bind(&parent)
    .bind(sort)
    .bind(id)
    .execute(pool)
    .await?;

    find_by_id(pool, id).await
}

/// 删除分类
///
/// 若分类不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &sqlx::SqlitePool, id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM categories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("category".into()));
    }
    Ok(())
}

//! 标签模型与数据库查询
//!
//! 定义标签（Tag）的数据结构以及对 `tags` 表的增删改查操作。
//! 标签通过 `posts_tags` 关联表与文章建立多对多关系。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::errors::app_error::{AppError, AppResult};

/// 标签完整数据库行模型
///
/// 直接映射 `tags` 表的所有字段。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub created_at: String,
}

/// 查询所有标签
///
/// 按 `name` 字母顺序排列返回完整标签列表。
pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<Tag>> {
    let tags = sqlx::query_as::<_, Tag>("SELECT * FROM tags ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(tags)
}

/// 根据标签 ID 查找标签
///
/// 若未找到则返回 [`AppError::NotFound`]。
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Tag> {
    let sql = crate::db::dialect::translate("SELECT * FROM tags WHERE id = ?");
    sqlx::query_as::<_, Tag>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// 创建新标签
///
/// 自动生成 UUID v7 作为主键。创建完成后重新查询并返回完整标签记录。
pub async fn create(pool: &crate::db::Pool, name: &str, slug: &str) -> AppResult<Tag> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        "INSERT INTO tags (id, name, slug, created_at) VALUES (?, ?, ?, ?)",
        id,
        name,
        slug,
        now,
    )
    .execute(pool)
    .await?;

    find_by_id(pool, &id).await
}

/// 删除标签
///
/// 若标签不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let result = sqlx::query!("DELETE FROM tags WHERE id = ?", id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("tag".into()));
    }
    Ok(())
}

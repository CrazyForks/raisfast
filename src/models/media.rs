//! 媒体文件模型与数据库查询
//!
//! 定义媒体文件（Media）的数据结构，包括完整行模型、面向前端的响应模型，
//! 以及对 `media` 表的增删改查操作。
//!
//! 响应模型中的 URL 由 `to_response()` 方法根据服务器地址动态拼接生成。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::errors::app_error::{AppError, AppResult};

/// 媒体文件完整数据库行模型
///
/// 直接映射 `media` 表的所有字段。
/// `filepath` 存储相对于上传目录的相对路径。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Media {
    pub id: String,
    pub user_id: String,
    pub filename: String,
    pub filepath: String,
    pub mimetype: String,
    pub size: i64,
    pub created_at: String,
}

/// 媒体文件 API 响应模型
///
/// 将 `filepath` 转换为完整的访问 URL，便于前端直接使用。
#[derive(Debug, Serialize)]
pub struct MediaResponse {
    pub id: String,
    pub user_id: String,
    pub filename: String,
    pub url: String,
    pub mimetype: String,
    pub size: i64,
    pub created_at: String,
}

impl Media {
    /// 将数据库行模型转换为响应模型
    ///
    /// `base_url` 为服务器地址（如 `http://localhost:3000`），
    /// 拼接 `/uploads/` 前缀和文件路径生成完整访问 URL。
    pub fn to_response(&self, base_url: &str) -> MediaResponse {
        MediaResponse {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            filename: self.filename.clone(),
            url: format!("{}/uploads/{}", base_url, self.filepath),
            mimetype: self.mimetype.clone(),
            size: self.size,
            created_at: self.created_at.clone(),
        }
    }
}

/// 创建媒体文件记录
///
/// 自动生成 UUID v7 作为主键。
/// 创建完成后重新查询并返回完整媒体文件记录。
pub async fn create(
    pool: &crate::db::Pool,
    user_id: &str,
    filename: &str,
    filepath: &str,
    mimetype: &str,
    size: i64,
) -> AppResult<Media> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        "INSERT INTO media (id, user_id, filename, filepath, mimetype, size, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        id,
        user_id,
        filename,
        filepath,
        mimetype,
        size,
        now,
    )
    .execute(pool)
    .await?;

    let sql = crate::db::dialect::translate("SELECT * FROM media WHERE id = ?");
    let media = sqlx::query_as::<_, Media>(&sql)
        .bind(&id)
        .fetch_one(pool)
        .await?;

    Ok(media)
}

/// 分页查询指定用户的媒体文件
///
/// 按 `created_at` 降序排列。返回媒体文件列表和总记录数。
pub async fn find_all(
    pool: &crate::db::Pool,
    user_id: &str,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Media>, i64)> {
    let offset = (page - 1) * page_size;
    let sql = crate::db::dialect::translate(
        "SELECT * FROM media WHERE user_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
    );
    let items = sqlx::query_as::<_, Media>(&sql)
        .bind(user_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    let sql = crate::db::dialect::translate("SELECT COUNT(*) FROM media WHERE user_id = ?");
    let total: (i64,) = sqlx::query_as(&sql).bind(user_id).fetch_one(pool).await?;

    Ok((items, total.0))
}

/// 根据媒体文件 ID 查找
///
/// 返回 `Ok(Some(media))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Option<Media>> {
    let sql = crate::db::dialect::translate("SELECT * FROM media WHERE id = ?");
    let media = sqlx::query_as::<_, Media>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(media)
}

/// 删除媒体文件记录
///
/// 仅删除数据库记录，不删除磁盘文件。
/// 若记录不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let result = sqlx::query!("DELETE FROM media WHERE id = ?", id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("media".into()));
    }
    Ok(())
}

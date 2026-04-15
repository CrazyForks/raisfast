//! 媒体文件模型与数据库查询
//!
//! 定义媒体文件（Media）的数据结构，包括完整行模型、面向前端的响应模型，
//! 以及对 `media` 表的增删改查操作。
//!
//! 响应模型中的 URL 由 `to_response()` 方法根据服务器地址动态拼接生成。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::{AppError, AppResult};
use crate::db::tenant::{resolve_tenant, tenant_filter};

/// 媒体文件完整数据库行模型
///
/// 直接映射 `media` 表的所有字段。
/// `filepath` 存储相对于上传目录的相对路径。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Media {
    pub id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub filename: String,
    pub filepath: String,
    pub mimetype: String,
    pub size: i64,
    pub created_at: String,
}

/// 创建媒体文件记录
///
/// 自动生成 UUID v7 作为主键。
/// 创建完成后重新查询并返回完整媒体文件记录。
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateMediaCmd,
    tenant_id: Option<&str>,
) -> AppResult<Media> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();
    let tid = resolve_tenant(tenant_id);

    let sql = crate::db::dialect::translate(
        "INSERT INTO media (id, tenant_id, user_id, filename, filepath, mimetype, size, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(tid)
        .bind(&cmd.user_id)
        .bind(&cmd.filename)
        .bind(&cmd.filepath)
        .bind(&cmd.mimetype)
        .bind(cmd.size)
        .bind(&now)
        .execute(pool)
        .await?;

    let sql_str = format!(
        "SELECT * FROM media WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Media>(&sql).bind(&id);
    if let Some(t) = tenant_id {
        q = q.bind(t);
    }
    let media = q.fetch_one(pool).await?;

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
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Media>, i64)> {
    let offset = (page - 1) * page_size;
    let sql_str = format!(
        "SELECT * FROM media WHERE user_id = ?{} ORDER BY created_at DESC LIMIT ? OFFSET ?",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Media>(&sql).bind(user_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q = q.bind(page_size).bind(offset);
    let items = q.fetch_all(pool).await?;

    let sql_str = format!(
        "SELECT COUNT(*) FROM media WHERE user_id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q2 = sqlx::query_as::<_, (i64,)>(&sql).bind(user_id);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    let total: (i64,) = q2.fetch_one(pool).await?;

    Ok((items, total.0))
}

/// 根据媒体文件 ID 查找
///
/// 返回 `Ok(Some(media))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Media>> {
    let sql_str = format!(
        "SELECT * FROM media WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, Media>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let media = q.fetch_optional(pool).await?;
    Ok(media)
}

/// 删除媒体文件记录
///
/// 仅删除数据库记录，不删除磁盘文件。
/// 若记录不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!("DELETE FROM media WHERE id = ?{}", tenant_filter(tenant_id));
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "media")
}

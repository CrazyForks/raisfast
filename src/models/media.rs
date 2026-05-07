//! 媒体文件模型与数据库查询
//!
//! 定义媒体文件（Media）的数据结构，包括完整行模型、面向前端的响应模型，
//! 以及对 `media` 表的增删改查操作。
//!
//! 响应模型中的 URL 由 `to_response()` 方法根据服务器地址动态拼接生成。

use serde::{Deserialize, Serialize};

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::{AppError, AppResult};

/// 媒体文件完整数据库行模型
///
/// 直接映射 `media` 表的所有字段。
/// `filepath` 存储相对于上传目录的相对路径。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Media {
    pub id: String,
    pub tenant_id: Option<String>,
    pub user_id: String,
    pub filename: String,
    pub filepath: String,
    pub mimetype: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: String,
}

crate::impl_from_row_opt_tenant!(Media {
    required { id, user_id, filename, filepath, mimetype, size, created_at }
    optional { width, height }
});

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

    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO media (id, tenant_id, user_id, filename, filepath, mimetype, size, width, height, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10)
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(tid)
                .bind(&cmd.user_id)
                .bind(&cmd.filename)
                .bind(&cmd.filepath)
                .bind(&cmd.mimetype)
                .bind(cmd.size)
                .bind(cmd.width)
                .bind(cmd.height)
                .bind(&now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO media (id, user_id, filename, filepath, mimetype, size, width, height, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9)
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(&cmd.user_id)
                .bind(&cmd.filename)
                .bind(&cmd.filepath)
                .bind(&cmd.mimetype)
                .bind(cmd.size)
                .bind(cmd.width)
                .bind(cmd.height)
                .bind(&now)
                .execute(pool)
                .await?;
        }
    }

    let sql = format!(
        "SELECT * FROM media WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
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
    let base = usize::from(tenant_id.is_some()) + 1;
    let sql = format!(
        "SELECT * FROM media WHERE user_id = {}{} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        ph(1),
        tenant_filter_ph(tenant_id, 2),
        ph(base + 1),
        ph(base + 2)
    );
    let mut q = sqlx::query_as::<_, Media>(&sql).bind(user_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q = q.bind(page_size).bind(offset);
    let items = q.fetch_all(pool).await?;

    let sql2 = format!(
        "SELECT COUNT(*) FROM media WHERE user_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q2 = sqlx::query_as::<_, (i64,)>(&sql2).bind(user_id);
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
    let sql = format!(
        "SELECT * FROM media WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Media>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let media = q.fetch_optional(pool).await?;
    Ok(media)
}

/// 存储统计信息
#[derive(Debug, Serialize, Clone)]
pub struct MediaStats {
    pub total_files: i64,
    pub total_size: i64,
    pub by_type: Vec<MediaTypeInfo>,
}

/// 按 MIME 类型分组的统计
#[derive(Debug, Serialize, Clone)]
pub struct MediaTypeInfo {
    pub mimetype: String,
    pub count: i64,
    pub total_size: i64,
}

/// 获取存储统计信息
pub async fn stats(
    pool: &crate::db::Pool,
    user_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<MediaStats> {
    let filter = tenant_filter_ph(tenant_id, 2);

    let total_sql = format!(
        "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM media WHERE user_id = {}{filter}",
        ph(1)
    );
    let mut q = sqlx::query_as::<_, (i64, i64)>(&total_sql).bind(user_id);
    if let Some(t) = tenant_id {
        q = q.bind(t);
    }
    let (total_files, total_size) = q.fetch_one(pool).await?;

    let by_type_sql = format!(
        "SELECT mimetype, COUNT(*), COALESCE(SUM(size), 0) FROM media WHERE user_id = {}{filter} GROUP BY mimetype ORDER BY COUNT(*) DESC",
        ph(1)
    );
    let mut q2 = sqlx::query_as::<_, (String, i64, i64)>(&by_type_sql).bind(user_id);
    if let Some(t) = tenant_id {
        q2 = q2.bind(t);
    }
    let rows = q2.fetch_all(pool).await?;

    let by_type = rows
        .into_iter()
        .map(|(mimetype, count, total_size)| MediaTypeInfo {
            mimetype,
            count,
            total_size,
        })
        .collect();

    Ok(MediaStats {
        total_files,
        total_size,
        by_type,
    })
}

/// 删除媒体文件记录
///
/// 仅删除数据库记录，不删除磁盘文件。
/// 若记录不存在则返回 [`AppError::NotFound`]。
pub async fn delete(pool: &crate::db::Pool, id: &str, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM media WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "media")
}

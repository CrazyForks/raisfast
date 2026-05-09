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
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Media {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub user_id: i64,
    pub filename: String,
    pub filepath: String,
    pub mimetype: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub title: Option<String>,
    pub alt_text: Option<String>,
    pub caption: Option<String>,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(Media {
    required { id, document_id, user_id, filename, filepath, mimetype, size, created_at, updated_at }
    optional { width, height, title, alt_text, caption, description }
});

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateMediaCmd,
    tenant_id: Option<&str>,
) -> AppResult<Media> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO media (document_id, tenant_id, user_id, filename, filepath, mimetype, size, width, height, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
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
                .bind(&document_id)
                .bind(tid)
                .bind(cmd.user_id)
                .bind(&cmd.filename)
                .bind(&cmd.filepath)
                .bind(&cmd.mimetype)
                .bind(cmd.size)
                .bind(cmd.width)
                .bind(cmd.height)
                .bind(now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO media (document_id, user_id, filename, filepath, mimetype, size, width, height, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
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
                .bind(&document_id)
                .bind(cmd.user_id)
                .bind(&cmd.filename)
                .bind(&cmd.filepath)
                .bind(&cmd.mimetype)
                .bind(cmd.size)
                .bind(cmd.width)
                .bind(cmd.height)
                .bind(now)
                .execute(pool)
                .await?;
        }
    }

    let sql = format!(
        "SELECT * FROM media WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Media>(&sql).bind(&document_id);
    if let Some(t) = tenant_id {
        q = q.bind(t);
    }
    let media = q.fetch_one(pool).await?;

    Ok(media)
}

pub async fn find_all(
    pool: &crate::db::Pool,
    user_id: i64,
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

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
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

#[derive(Debug, Serialize, Clone)]
pub struct MediaStats {
    pub total_files: i64,
    pub total_size: i64,
    pub by_type: Vec<MediaTypeInfo>,
}

#[derive(Debug, Serialize, Clone)]
pub struct MediaTypeInfo {
    pub mimetype: String,
    pub count: i64,
    pub total_size: i64,
}

pub async fn stats(
    pool: &crate::db::Pool,
    user_id: i64,
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

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::media::CreateMediaCmd;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let doc_id = crate::utils::id::new_document_id();
        let sql = format!(
            "INSERT INTO users (document_id, email, username, password_hash, role) VALUES ({}, {}, {}, {}, 'admin') RETURNING id",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2),
            crate::db::dialect::ph(3),
            crate::db::dialect::ph(4)
        );
        let (id,): (i64,) = sqlx::query_as(&sql)
            .bind(&doc_id)
            .bind("media-test@test.com")
            .bind("mediauser")
            .bind("$argon2id$v=19$m=19456,t=2,p=1$test$test")
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    fn make_cmd(user_id: i64, filename: &str) -> CreateMediaCmd {
        CreateMediaCmd {
            user_id,
            filename: filename.to_string(),
            filepath: format!("/uploads/{filename}"),
            mimetype: "image/png".to_string(),
            size: 1024,
            width: Some(100),
            height: Some(100),
        }
    }

    #[tokio::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        let media = create(&pool, &make_cmd(uid, "photo.png"), None)
            .await
            .unwrap();
        let found = find_by_id(&pool, media.id, None).await.unwrap().unwrap();
        assert_eq!(found.id, media.id);
        assert_eq!(found.filename, "photo.png");
        assert_eq!(found.user_id, uid);
    }

    #[tokio::test]
    async fn find_all_paginated() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        for i in 0..5 {
            create(&pool, &make_cmd(uid, &format!("file{i}.png")), None)
                .await
                .unwrap();
        }
        let (items, total) = find_all(&pool, uid, 1, 3, None).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn stats_returns_counts() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        for i in 0..3 {
            create(&pool, &make_cmd(uid, &format!("img{i}.png")), None)
                .await
                .unwrap();
        }
        let s = stats(&pool, uid, None).await.unwrap();
        assert_eq!(s.total_files, 3);
        assert_eq!(s.total_size, 3 * 1024);
        assert_eq!(s.by_type.len(), 1);
        assert_eq!(s.by_type[0].mimetype, "image/png");
        assert_eq!(s.by_type[0].count, 3);
    }

    #[tokio::test]
    async fn delete_removes_media() {
        let pool = setup_pool().await;
        let uid = insert_user(&pool).await;
        let media = create(&pool, &make_cmd(uid, "gone.png"), None)
            .await
            .unwrap();
        delete(&pool, media.id, None).await.unwrap();
        let found = find_by_id(&pool, media.id, None).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        let found = find_by_id(&pool, 99999, None).await.unwrap();
        assert!(found.is_none());
    }
}

//! 分类模型与数据库查询
//!
//! 定义内容分类的数据结构以及对 `categories` 表的增删改查操作。
//! 分类支持嵌套（通过 `parent_id`）和自定义排序（通过 `sort_order`）。

use serde::{Deserialize, Serialize};

#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Category {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
    pub updated_by: Option<i64>,
    pub updated_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(Category {
    required { id, document_id, name, slug, sort_order, created_at }
    optional { description, parent_id, updated_by, updated_at }
});

pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<Category>> {
    let sql = format!(
        "SELECT * FROM categories WHERE 1=1{} ORDER BY sort_order, name",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut q = sqlx::query_as::<_, Category>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let categories = q.fetch_all(pool).await?;
    Ok(categories)
}

pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Category>, i64)> {
    let offset = (page - 1).max(0) * page_size;

    let count_sql = format!(
        "SELECT COUNT(*) FROM categories WHERE 1=1{}",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let total = cq.fetch_one(pool).await?;

    let base = usize::from(tenant_id.is_some()) + 1;
    let data_sql = format!(
        "SELECT * FROM categories WHERE 1=1{} ORDER BY sort_order, name LIMIT {} OFFSET {}",
        tenant_filter_ph(tenant_id, 1),
        ph(base),
        ph(base + 1)
    );
    let mut dq = sqlx::query_as::<_, Category>(&data_sql);
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
) -> AppResult<Category> {
    let sql = format!(
        "SELECT * FROM categories WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Category>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Category>> {
    let sql = format!(
        "SELECT * FROM categories WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Category>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateCategoryCmd,
    tenant_id: Option<&str>,
    created_by: Option<i64>,
) -> AppResult<Category> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO categories (document_id, tenant_id, name, slug, description, parent_id, sort_order, created_by, updated_by, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10),
                ph(11)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(&cmd.name)
                .bind(&cmd.slug)
                .bind(&cmd.description)
                .bind(cmd.parent_id)
                .bind(cmd.sort_order)
                .bind(created_by)
                .bind(created_by)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO categories (document_id, name, slug, description, parent_id, sort_order, created_by, updated_by, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
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
                .bind(&cmd.name)
                .bind(&cmd.slug)
                .bind(&cmd.description)
                .bind(cmd.parent_id)
                .bind(cmd.sort_order)
                .bind(created_by)
                .bind(created_by)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await?;
        }
    }

    find_by_document_id(pool, &document_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch created category")))
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateCategoryCmd,
    tenant_id: Option<&str>,
    updated_by: Option<i64>,
) -> AppResult<Category> {
    let cat_id: i64 = cmd.id;
    let existing = find_by_id(pool, cat_id, tenant_id).await?;

    let name = cmd.name.as_deref().unwrap_or(&existing.name);
    let slug = cmd.slug.as_deref().unwrap_or(&existing.slug);
    let desc = cmd
        .description
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(existing.description);
    let parent = cmd.parent_id.or(existing.parent_id);
    let sort = cmd.sort_order.unwrap_or(existing.sort_order);

    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE categories SET name = {}, slug = {}, description = {}, parent_id = {}, sort_order = {}, updated_by = {}, updated_at = {} WHERE id = {}{}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        tenant_filter_ph(tenant_id, 9)
    );
    let mut q = sqlx::query(&sql)
        .bind(name)
        .bind(slug)
        .bind(desc)
        .bind(parent)
        .bind(sort)
        .bind(updated_by)
        .bind(now)
        .bind(cat_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;

    find_by_id(pool, cat_id, tenant_id).await
}

pub async fn delete(pool: &crate::db::Pool, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM categories WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "category")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::category::CreateCategoryCmd;
    use crate::commands::category::UpdateCategoryCmd;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn make_cmd(name: &str) -> CreateCategoryCmd {
        CreateCategoryCmd {
            name: name.to_string(),
            slug: name.to_lowercase(),
            description: None,
            parent_id: None,
            sort_order: 0,
        }
    }

    #[tokio::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let cat = create(&pool, &make_cmd("Tech"), None, None).await.unwrap();
        let found = find_by_id(&pool, cat.id, None).await.unwrap();
        assert_eq!(found.id, cat.id);
        assert_eq!(found.name, "Tech");
    }

    #[tokio::test]
    async fn find_by_document_id() {
        let pool = setup_pool().await;
        let cat = create(&pool, &make_cmd("Science"), None, None)
            .await
            .unwrap();
        let found = super::find_by_document_id(&pool, &cat.document_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, cat.id);
    }

    #[tokio::test]
    async fn find_all_returns_all() {
        let pool = setup_pool().await;
        create(&pool, &make_cmd("A"), None, None).await.unwrap();
        create(&pool, &make_cmd("B"), None, None).await.unwrap();
        create(&pool, &make_cmd("C"), None, None).await.unwrap();
        let all = find_all(&pool, None).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn find_paginated() {
        let pool = setup_pool().await;
        for name in ["A", "B", "C", "D", "E"] {
            create(&pool, &make_cmd(name), None, None).await.unwrap();
        }
        let (items, total) = super::find_paginated(&pool, None, 1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn update_changes_name() {
        let pool = setup_pool().await;
        let cat = create(&pool, &make_cmd("Old"), None, None).await.unwrap();
        let updated = update(
            &pool,
            &UpdateCategoryCmd {
                id: cat.id,
                name: Some("New".to_string()),
                slug: None,
                description: None,
                parent_id: None,
                sort_order: None,
            },
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "New");
    }

    #[tokio::test]
    async fn delete_removes_category() {
        let pool = setup_pool().await;
        let cat = create(&pool, &make_cmd("Gone"), None, None).await.unwrap();
        delete(&pool, cat.id, None).await.unwrap();
        let result = find_by_id(&pool, cat.id, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        let result = find_by_id(&pool, 99999, None).await;
        assert!(result.is_err());
    }
}

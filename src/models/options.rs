//! Site configuration model and database queries
//!
//! Each row in the `options` table contains full metadata (type, group, label, validation rules).
//! Reads can be returned directly to the frontend for rendering grouped forms.

use serde::{Deserialize, Serialize};

use crate::constants::COL_TENANT_ID;
use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

define_enum!(
    OptionType {
        Text = "text",
        Url = "url",
        Email = "email",
        Select = "select",
        Integer = "integer",
        Boolean = "boolean",
    }
);

/// Options table row model (with full metadata)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OptionRow {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<i64>,
    pub option_key: String,
    pub value: String,
    #[serde(rename = "type")]
    pub type_: OptionType,
    pub group_name: String,
    pub label: String,
    pub description: Option<String>,
    pub validation: Option<String>,
    pub is_public: bool,
    pub autoload: bool,
    pub sort_order: i64,
    pub updated_at: Timestamp,
}

#[cfg(feature = "db-sqlite")]
impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for OptionRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            document_id: row.try_get("document_id")?,
            tenant_id: row.try_get("tenant_id").ok(),
            option_key: row.try_get("option_key")?,
            value: row.try_get("value")?,
            type_: row.try_get("type")?,
            group_name: row.try_get("group_name")?,
            label: row.try_get("label")?,
            description: row.try_get("description")?,
            validation: row.try_get("validation")?,
            is_public: row.try_get("is_public")?,
            autoload: row.try_get("autoload")?,
            sort_order: row.try_get("sort_order")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}
#[cfg(feature = "db-postgres")]
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for OptionRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            document_id: row.try_get("document_id")?,
            tenant_id: row.try_get("tenant_id").ok(),
            option_key: row.try_get("option_key")?,
            value: row.try_get("value")?,
            type_: row.try_get("type")?,
            group_name: row.try_get("group_name")?,
            label: row.try_get("label")?,
            description: row.try_get("description")?,
            validation: row.try_get("validation")?,
            is_public: row.try_get("is_public")?,
            autoload: row.try_get("autoload")?,
            sort_order: row.try_get("sort_order")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}
#[cfg(feature = "db-mysql")]
impl<'r> sqlx::FromRow<'r, sqlx::mysql::MySqlRow> for OptionRow {
    fn from_row(row: &'r sqlx::mysql::MySqlRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            document_id: row.try_get("document_id")?,
            tenant_id: row.try_get("tenant_id").ok(),
            option_key: row.try_get("option_key")?,
            value: row.try_get("value")?,
            type_: row.try_get("type")?,
            group_name: row.try_get("group_name")?,
            label: row.try_get("label")?,
            description: row.try_get("description")?,
            validation: row.try_get("validation")?,
            is_public: row.try_get("is_public")?,
            autoload: row.try_get("autoload")?,
            sort_order: row.try_get("sort_order")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

/// Query all autoload options (preloaded at startup)
pub async fn find_autoload(pool: &crate::db::Pool) -> AppResult<Vec<OptionRow>> {
    let sql = "SELECT * FROM options WHERE autoload = 1";
    let rows = sqlx::query_as::<_, OptionRow>(sql).fetch_all(pool).await?;
    Ok(rows)
}

/// Query a single option by key
pub async fn find_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<i64>,
) -> AppResult<Option<OptionRow>> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "SELECT * FROM options WHERE {COL_TENANT_ID} = {} AND option_key = {}",
                ph(1),
                ph(2)
            );
            let row = sqlx::query_as::<_, OptionRow>(&sql)
                .bind(tid)
                .bind(key)
                .fetch_optional(pool)
                .await?;
            Ok(row)
        }
        None => {
            let sql = format!("SELECT * FROM options WHERE option_key = {}", ph(1));
            let row = sqlx::query_as::<_, OptionRow>(&sql)
                .bind(key)
                .fetch_optional(pool)
                .await?;
            Ok(row)
        }
    }
}

/// Query all options
pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<i64>) -> AppResult<Vec<OptionRow>> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "SELECT * FROM options WHERE {COL_TENANT_ID} = {} ORDER BY sort_order, option_key",
                ph(1)
            );
            let rows = sqlx::query_as::<_, OptionRow>(&sql)
                .bind(tid)
                .fetch_all(pool)
                .await?;
            Ok(rows)
        }
        None => {
            let sql = "SELECT * FROM options ORDER BY sort_order, option_key";
            let rows = sqlx::query_as::<_, OptionRow>(sql).fetch_all(pool).await?;
            Ok(rows)
        }
    }
}

/// Insert or update an option value (UPSERT by key)
pub async fn upsert_value(
    pool: &crate::db::Pool,
    key: &str,
    value: &str,
    tenant_id: Option<i64>,
) -> AppResult<()> {
    match tenant_id {
        Some(tid) => {
            let now = crate::utils::tz::now_utc();
            let sql = format!(
                "UPDATE options SET value = {}, updated_at = {} WHERE {COL_TENANT_ID} = {} AND option_key = {}",
                ph(1),
                ph(2),
                ph(3),
                ph(4)
            );
            sqlx::query(&sql)
                .bind(value)
                .bind(now)
                .bind(tid)
                .bind(key)
                .execute(pool)
                .await?;
        }
        None => {
            let now = crate::utils::tz::now_utc();
            let sql = format!(
                "UPDATE options SET value = {}, updated_at = {} WHERE option_key = {}",
                ph(1),
                ph(2),
                ph(3)
            );
            sqlx::query(&sql)
                .bind(value)
                .bind(now)
                .bind(key)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// Delete an option by key
pub async fn delete_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<i64>,
) -> AppResult<()> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "DELETE FROM options WHERE {COL_TENANT_ID} = {} AND option_key = {}",
                ph(1),
                ph(2)
            );
            sqlx::query(&sql).bind(tid).bind(key).execute(pool).await?;
        }
        None => {
            let sql = format!("DELETE FROM options WHERE option_key = {}", ph(1));
            sqlx::query(&sql).bind(key).execute(pool).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn insert_test_option(
        pool: &crate::db::Pool,
        key: &str,
        value: &str,
        autoload: bool,
        updated_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO options (document_id, option_key, value, type, group_name, label, autoload, sort_order, updated_at) \
             VALUES (?, ?, ?, ?, 'test', 'test', ?, 0, ?)",
        )
        .bind(crate::utils::id::new_document_id())
        .bind(key)
        .bind(value)
        .bind(OptionType::Text)
        .bind(autoload)
        .bind(updated_at)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn upsert_and_find_by_key() {
        let pool = setup_pool().await;
        let key = format!("test.{}", crate::utils::id::new_document_id());
        let now_str = crate::utils::tz::now_utc().to_rfc3339();

        insert_test_option(&pool, &key, "initial", true, &now_str).await;
        upsert_value(&pool, &key, "updated", None).await.unwrap();

        let found = find_by_key(&pool, &key, None).await.unwrap();
        assert!(found.is_some());
        let row = found.unwrap();
        assert_eq!(row.option_key, key);
        assert_eq!(row.value, "updated");
    }

    #[tokio::test]
    async fn find_all_returns_all() {
        let pool = setup_pool().await;
        let now = crate::utils::tz::now_utc();
        let now_str = now.to_rfc3339();

        let k1 = format!("test.{}", crate::utils::id::new_document_id());
        let k2 = format!("test.{}", crate::utils::id::new_document_id());
        let k3 = format!("test.{}", crate::utils::id::new_document_id());
        insert_test_option(&pool, &k1, "v1", true, &now_str).await;
        insert_test_option(&pool, &k2, "v2", true, &now_str).await;
        insert_test_option(&pool, &k3, "v3", true, &now_str).await;

        let all = find_all(&pool, None).await.unwrap();
        assert!(all.len() >= 3);
    }

    #[tokio::test]
    async fn upsert_overwrites() {
        let pool = setup_pool().await;
        let key = format!("test.{}", crate::utils::id::new_document_id());
        let now = crate::utils::tz::now_utc();
        let now_str = now.to_rfc3339();

        insert_test_option(&pool, &key, "v1", true, &now_str).await;
        upsert_value(&pool, &key, "v2", None).await.unwrap();

        let found = find_by_key(&pool, &key, None).await.unwrap().unwrap();
        assert_eq!(found.value, "v2");
    }

    #[tokio::test]
    async fn delete_by_key_removes() {
        let pool = setup_pool().await;
        let key = format!("test.{}", crate::utils::id::new_document_id());
        let now_str = crate::utils::tz::now_utc().to_rfc3339();

        insert_test_option(&pool, &key, "val", true, &now_str).await;
        delete_by_key(&pool, &key, None).await.unwrap();

        let found = find_by_key(&pool, &key, None).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_autoload_test() {
        let pool = setup_pool().await;
        let now_str = crate::utils::tz::now_utc().to_rfc3339();

        let k1 = format!("test.{}", crate::utils::id::new_document_id());
        let k2 = format!("test.{}", crate::utils::id::new_document_id());
        let k3 = format!("test.{}", crate::utils::id::new_document_id());
        insert_test_option(&pool, &k1, "v1", true, &now_str).await;
        insert_test_option(&pool, &k2, "v2", true, &now_str).await;
        insert_test_option(&pool, &k3, "v3", false, &now_str).await;

        let autoloaded = find_autoload(&pool).await.unwrap();
        assert!(autoloaded.iter().any(|r| r.option_key == k1));
        assert!(autoloaded.iter().any(|r| r.option_key == k2));
        assert!(!autoloaded.iter().any(|r| r.option_key == k3));
    }
}

//! Site configuration model and database queries
//!
//! Each row in the `options` table contains full metadata (type, group, label, validation rules).
//! Reads can be returned directly to the frontend for rendering grouped forms.

use serde::{Deserialize, Serialize};

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
    pub tenant_id: Option<String>,
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
    Ok(raisfast_derive::crud_find_all!(pool, "options", OptionRow, "autoload" => 1_i64)?)
}

/// Query a single option by key
pub async fn find_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<OptionRow>> {
    Ok(
        raisfast_derive::crud_find!(pool, "options", OptionRow, "option_key" => key, tenant: tenant_id)?,
    )
}

/// Query all options
pub async fn find_all(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<OptionRow>> {
    Ok(
        raisfast_derive::crud_list!(pool, "options", OptionRow, order_by: "sort_order, option_key", tenant: tenant_id)?,
    )
}

/// Insert or update an option value (UPSERT by key)
pub async fn upsert_value(
    pool: &crate::db::Pool,
    key: &str,
    value: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "options",
        bind: ["value" => value, "updated_at" => now],
        where: "option_key" => key,
        tenant: tenant_id
    )?;
    Ok(())
}

/// Delete an option by key
pub async fn delete_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "options", "option_key" => key, tenant: tenant_id)?;
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

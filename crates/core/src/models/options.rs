//! Site configuration model and database queries
//!
//! Each row in the `options` table contains full metadata (type, group, label, validation rules).
//! Reads can be returned directly to the frontend for rendering grouped forms.

use serde::{Deserialize, Serialize};

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
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
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct OptionRow {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub option_key: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub value: serde_json::Value,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub type_: OptionType,
    pub group_name: String,
    pub label: String,
    pub description: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub validation: Option<serde_json::Value>,
    pub is_public: bool,
    pub autoload: bool,
    pub sort_order: i64,
    pub updated_at: Timestamp,
}

/// Query all autoload options (preloaded at startup)
pub async fn find_autoload(pool: &crate::db::Pool) -> AppResult<Vec<OptionRow>> {
    Ok(raisfast_derive::crud_find_all!(pool, "options", OptionRow, where: ("autoload", true))?)
}

/// Query a single option by key
pub async fn find_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<OptionRow>> {
    Ok(
        raisfast_derive::crud_find!(pool, "options", OptionRow, where: ("option_key", key), tenant: tenant_id)?,
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
    value: &serde_json::Value,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "options",
        bind: ["value" => value, "updated_at" => now],
        where: ("option_key", key),
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
    raisfast_derive::crud_delete!(pool, "options", where: ("option_key", key), tenant: tenant_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbDriver;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn insert_test_option(
        pool: &crate::db::Pool,
        key: &str,
        value: &str,
        autoload: bool,
        updated_at: &crate::utils::tz::Timestamp,
    ) {
        let id = crate::utils::id::new_id();
        sqlx::query(&format!(
            "INSERT INTO options (id, option_key, value, type, group_name, label, autoload, sort_order, updated_at) \
             VALUES ({}, {}, {}, {}, 'test', 'test', {}, 0, {})",
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3),
            crate::db::Driver::ph(4),
            crate::db::Driver::ph(5),
            crate::db::Driver::ph(6)
        ))
        .bind(id)
        .bind(key)
        .bind(serde_json::Value::String(value.to_string()))
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
        let key = format!("test.{}", crate::utils::id::new_id());
        let now = crate::utils::tz::now_utc();

        insert_test_option(&pool, &key, "initial", true, &now).await;
        upsert_value(&pool, &key, &serde_json::json!("updated"), None).await.unwrap();

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

        let k1 = format!("test.{}", crate::utils::id::new_id());
        let k2 = format!("test.{}", crate::utils::id::new_id());
        let k3 = format!("test.{}", crate::utils::id::new_id());
        insert_test_option(&pool, &k1, "v1", true, &now).await;
        insert_test_option(&pool, &k2, "v2", true, &now).await;
        insert_test_option(&pool, &k3, "v3", true, &now).await;

        let all = find_all(&pool, None).await.unwrap();
        assert!(all.len() >= 3);
    }

    #[tokio::test]
    async fn upsert_overwrites() {
        let pool = setup_pool().await;
        let key = format!("test.{}", crate::utils::id::new_id());
        let now = crate::utils::tz::now_utc();

        insert_test_option(&pool, &key, "v1", true, &now).await;
        upsert_value(&pool, &key, &serde_json::json!("v2"), None).await.unwrap();

        let found = find_by_key(&pool, &key, None).await.unwrap().unwrap();
        assert_eq!(found.value, "v2");
    }

    #[tokio::test]
    async fn delete_by_key_removes() {
        let pool = setup_pool().await;
        let key = format!("test.{}", crate::utils::id::new_id());
        let now = crate::utils::tz::now_utc();

        insert_test_option(&pool, &key, "val", true, &now).await;
        delete_by_key(&pool, &key, None).await.unwrap();

        let found = find_by_key(&pool, &key, None).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_autoload_test() {
        let pool = setup_pool().await;
        let now = crate::utils::tz::now_utc();

        let k1 = format!("test.{}", crate::utils::id::new_id());
        let k2 = format!("test.{}", crate::utils::id::new_id());
        let k3 = format!("test.{}", crate::utils::id::new_id());
        insert_test_option(&pool, &k1, "v1", true, &now).await;
        insert_test_option(&pool, &k2, "v2", true, &now).await;
        insert_test_option(&pool, &k3, "v3", false, &now).await;

        let autoloaded = find_autoload(&pool).await.unwrap();
        assert!(autoloaded.iter().any(|r| r.option_key == k1));
        assert!(autoloaded.iter().any(|r| r.option_key == k2));
        assert!(!autoloaded.iter().any(|r| r.option_key == k3));
    }
}

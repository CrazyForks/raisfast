use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Currency {
    pub id: i64,
    pub document_id: String,
    pub code: String,
    pub name: String,
    pub decimals: i64,
    pub is_active: bool,
    pub version: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_code(pool: &crate::db::Pool, code: &str) -> AppResult<Option<Currency>> {
    let sql = format!("SELECT * FROM currencies WHERE code = {}", ph(1));
    sqlx::query_as::<_, Currency>(&sql)
        .bind(code)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_active_by_code(
    pool: &crate::db::Pool,
    code: &str,
) -> AppResult<Option<Currency>> {
    let sql = format!(
        "SELECT * FROM currencies WHERE code = {} AND is_active = 1",
        ph(1)
    );
    sqlx::query_as::<_, Currency>(&sql)
        .bind(code)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_by_code_tx(
    tx: &mut crate::db::pool::DbConnection,
    code: &str,
) -> AppResult<Option<Currency>> {
    let sql = format!(
        "SELECT * FROM currencies WHERE code = {} AND is_active = 1",
        ph(1)
    );
    sqlx::query_as::<_, Currency>(&sql)
        .bind(code)
        .fetch_optional(tx)
        .await
        .map_err(Into::into)
}

pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<Currency>> {
    let sql = "SELECT * FROM currencies ORDER BY code";
    sqlx::query_as::<_, Currency>(sql)
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub async fn create(
    pool: &crate::db::Pool,
    code: &str,
    name: &str,
    decimals: i64,
) -> AppResult<Currency> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO currencies (document_id, code, name, decimals, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6)
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(code)
        .bind(name)
        .bind(decimals)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

    let sql = format!("SELECT * FROM currencies WHERE document_id = {}", ph(1));
    sqlx::query_as::<_, Currency>(&sql)
        .bind(&document_id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn update(
    pool: &crate::db::Pool,
    code: &str,
    name: Option<&str>,
    is_active: Option<bool>,
) -> AppResult<Option<Currency>> {
    let existing = find_by_code(pool, code).await?;
    let existing = match existing {
        Some(e) => e,
        None => return Ok(None),
    };

    let name = name.unwrap_or(&existing.name);
    let is_active = is_active.unwrap_or(existing.is_active);
    let now = crate::utils::tz::now_str();

    let sql = format!(
        "UPDATE currencies SET name = {}, is_active = {}, version = version + 1, updated_at = {} WHERE id = {} AND version = {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5)
    );
    let affected = sqlx::query(&sql)
        .bind(name)
        .bind(is_active)
        .bind(now)
        .bind(existing.id)
        .bind(existing.version)
        .execute(pool)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(crate::errors::app_error::AppError::Conflict(
            "concurrent_currency_update".into(),
        ));
    }

    find_by_code(pool, code).await
}

pub async fn delete_by_code(pool: &crate::db::Pool, code: &str) -> AppResult<bool> {
    let existing = find_by_code(pool, code).await?;
    let existing = match existing {
        Some(e) => e,
        None => return Ok(false),
    };

    let (count,): (i64,) = sqlx::query_as(&format!(
        "SELECT COUNT(*) as count FROM wallets WHERE currency = {}",
        ph(1)
    ))
    .bind(code)
    .fetch_one(pool)
    .await?;

    if count > 0 {
        return Err(crate::errors::app_error::AppError::BadRequest(format!(
            "currency_in_use: {count} wallet(s) using '{code}'"
        )));
    }

    let sql = format!("DELETE FROM currencies WHERE id = {}", ph(1));
    let affected = sqlx::query(&sql)
        .bind(existing.id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn create_and_find_currency() {
        let pool = setup_pool().await;
        let c = create(&pool, "CNY", "Chinese Yuan", 2).await.unwrap();
        assert_eq!(c.code, "CNY");
        assert_eq!(c.name, "Chinese Yuan");
        assert_eq!(c.decimals, 2);
        assert!(c.is_active);

        let found = find_by_code(&pool, "CNY").await.unwrap().unwrap();
        assert_eq!(found.document_id, c.document_id);
    }

    #[tokio::test]
    async fn find_nonexistent_returns_none() {
        let pool = setup_pool().await;
        assert!(find_by_code(&pool, "XXX").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_currency() {
        let pool = setup_pool().await;
        create(&pool, "USD", "US Dollar", 2).await.unwrap();

        let updated = update(&pool, "USD", Some("US Dollar Updated"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "US Dollar Updated");
        assert_eq!(updated.decimals, 2);
    }

    #[tokio::test]
    async fn deactivate_currency() {
        let pool = setup_pool().await;
        create(&pool, "EUR", "Euro", 2).await.unwrap();

        update(&pool, "EUR", None, Some(false)).await.unwrap();
        let c = find_by_code(&pool, "EUR").await.unwrap().unwrap();
        assert!(!c.is_active);

        assert!(find_active_by_code(&pool, "EUR").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_currency() {
        let pool = setup_pool().await;
        create(&pool, "JPY", "Japanese Yen", 0).await.unwrap();
        assert!(delete_by_code(&pool, "JPY").await.unwrap());
        assert!(find_by_code(&pool, "JPY").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_currency_in_use_rejected() {
        let pool = setup_pool().await;
        create(&pool, "CNY", "Chinese Yuan", 2).await.unwrap();
        let user = crate::models::user::create(
            &pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_document_id(),
                registered_via: crate::models::user::RegisteredVia::Email,
            },
            None,
        )
        .await
        .unwrap();
        crate::models::wallet::create(&pool, user.id, "CNY")
            .await
            .unwrap();

        let err = delete_by_code(&pool, "CNY").await.unwrap_err();
        match err {
            crate::errors::app_error::AppError::BadRequest(msg) => {
                assert!(msg.starts_with("currency_in_use"));
            }
            _ => panic!("expected BadRequest, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_false() {
        let pool = setup_pool().await;
        assert!(!delete_by_code(&pool, "XXX").await.unwrap());
    }

    #[tokio::test]
    async fn find_all_currencies() {
        let pool = setup_pool().await;
        create(&pool, "CNY", "Chinese Yuan", 2).await.unwrap();
        create(&pool, "USD", "US Dollar", 2).await.unwrap();
        let all = find_all(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].code, "CNY");
        assert_eq!(all[1].code, "USD");
    }
}

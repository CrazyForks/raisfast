use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    WalletStatus {
        Active = "active",
        Frozen = "frozen",
    }
);

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Wallet {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub currency: String,
    pub balance: i64,
    pub version: i64,
    pub status: WalletStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_user_and_currency(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    currency: &str,
) -> AppResult<Option<Wallet>> {
    let result: Option<Wallet> = raisfast_derive::crud_find!(pool, "wallets", Wallet, where: AND(("user_id", user_id), ("currency", currency)))?;
    Ok(result)
}

pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<Option<Wallet>> {
    let result: Option<Wallet> =
        raisfast_derive::crud_find!(pool, "wallets", Wallet, where: ("id", id))?;
    Ok(result)
}

pub async fn find_by_user(pool: &crate::db::Pool, user_id: SnowflakeId) -> AppResult<Vec<Wallet>> {
    let result: Vec<Wallet> =
        raisfast_derive::crud_find_all!(pool, "wallets", Wallet, where: ("user_id", user_id))?;
    Ok(result)
}

pub async fn create(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    currency: &str,
) -> AppResult<Wallet> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(pool, "wallets", [
        "id" => id,
        "user_id" => user_id,
        "currency" => currency,
        "created_at" => now,
        "updated_at" => now
    ])?;
    let result: Wallet =
        raisfast_derive::crud_find_one!(pool, "wallets", Wallet, where: ("id", id))?;
    Ok(result)
}

pub async fn find_or_create(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    currency: &str,
) -> AppResult<Wallet> {
    if let Some(w) = find_by_user_and_currency(pool, user_id, currency).await? {
        return Ok(w);
    }
    create(pool, user_id, currency).await
}

pub async fn find_all_wallets(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<Wallet>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, Wallet,
        table: "wallets",
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn tx_find_by_id(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
) -> AppResult<Option<Wallet>> {
    Ok(raisfast_derive::crud_find!(tx, "wallets", Wallet, where: ("id", id))?)
}

pub async fn tx_find_by_user_and_currency(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    currency: &str,
) -> AppResult<Option<Wallet>> {
    Ok(raisfast_derive::crud_find!(
        &mut *tx, "wallets", Wallet, where: AND(("user_id", user_id), ("currency", currency))
    )?)
}

pub async fn tx_create(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    currency: &str,
) -> AppResult<Wallet> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        &mut *tx, "wallets",
        ["id" => id, "user_id" => user_id, "currency" => currency, "created_at" => now, "updated_at" => now]
    )?;
    Ok(raisfast_derive::crud_find_one!(&mut *tx, "wallets", Wallet, where: ("id", id))?)
}

pub async fn tx_find_or_create(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    currency: &str,
) -> AppResult<Wallet> {
    if let Some(w) = tx_find_by_user_and_currency(tx, user_id, currency).await? {
        return Ok(w);
    }
    match tx_create(tx, user_id, currency).await {
        Ok(w) => Ok(w),
        Err(_) => Ok(raisfast_derive::crud_find_one!(
            &mut *tx, "wallets", Wallet, where: AND(("user_id", user_id), ("currency", currency))
        )?),
    }
}

pub async fn apply_wallet_delta(
    tx: &mut crate::db::pool::DbConnection,
    wallet_id: SnowflakeId,
    version: i64,
    delta: i64,
    current_balance: i64,
) -> AppResult<()> {
    use crate::db::Driver;
    use crate::db::driver::DbDriver;
    raisfast_derive::check_schema!("wallets", "balance", "version", "updated_at", "id");
    if delta > 0 {
        let _ = current_balance.checked_add(delta).ok_or_else(|| {
            crate::errors::app_error::AppError::BadRequest("balance_overflow".into())
        })?;
        let sql = format!(
            "UPDATE wallets SET balance = balance + {}, version = version + 1, updated_at = {} WHERE id = {} AND version = {}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(4)
        );
        let result: crate::db::pool::DbQueryResult = sqlx::query(&sql)
            .bind(delta)
            .bind(crate::utils::tz::now_str())
            .bind(wallet_id)
            .bind(version)
            .execute(&mut *tx)
            .await?;
        let affected = result.rows_affected();
        if affected == 0 {
            return Err(crate::errors::app_error::AppError::Conflict(
                "concurrent_wallet_update".into(),
            ));
        }
    } else {
        let abs = -delta;
        let sql = format!(
            "UPDATE wallets SET balance = balance - {}, version = version + 1, updated_at = {} WHERE id = {} AND balance >= {} AND version = {}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(4),
            Driver::ph(5)
        );
        let result: crate::db::pool::DbQueryResult = sqlx::query(&sql)
            .bind(abs)
            .bind(crate::utils::tz::now_str())
            .bind(wallet_id)
            .bind(abs)
            .bind(version)
            .execute(&mut *tx)
            .await?;
        let affected = result.rows_affected();
        if affected == 0 {
            return Err(crate::errors::app_error::AppError::BadRequest(
                "insufficient_balance_or_concurrent_update".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn insert_user(pool: &crate::db::Pool) -> crate::models::user::User {
        crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_id().to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_wallet() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let w = create(&pool, user.id, "CNY").await.unwrap();
        assert_eq!(w.user_id, user.id);
        assert_eq!(w.currency, "CNY");
        assert_eq!(w.balance, 0);
        assert_eq!(w.version, 1);
        assert_eq!(w.status, WalletStatus::Active);
    }

    #[tokio::test]
    async fn create_wallet_same_user_different_currency() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let w1 = create(&pool, user.id, "CNY").await.unwrap();
        let w2 = create(&pool, user.id, "USD").await.unwrap();
        assert_ne!(w1.id, w2.id);
        assert_eq!(w2.currency, "USD");
    }

    #[tokio::test]
    async fn find_by_id_found() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let w = create(&pool, user.id, "CNY").await.unwrap();
        let found = find_by_id(&pool, w.id).await.unwrap().unwrap();
        assert_eq!(found.id, w.id);
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            find_by_id(&pool, SnowflakeId(99999))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_by_user_and_currency_found() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        create(&pool, user.id, "CNY").await.unwrap();
        let found = find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.currency, "CNY");
    }

    #[tokio::test]
    async fn find_by_user_and_currency_wrong_currency() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        create(&pool, user.id, "CNY").await.unwrap();
        assert!(
            find_by_user_and_currency(&pool, user.id, "USD")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_by_user_returns_all_wallets() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        create(&pool, user.id, "CNY").await.unwrap();
        create(&pool, user.id, "USD").await.unwrap();
        let wallets = find_by_user(&pool, user.id).await.unwrap();
        assert_eq!(wallets.len(), 2);
    }

    #[tokio::test]
    async fn find_by_user_empty() {
        let pool = setup_pool().await;
        let wallets = find_by_user(&pool, SnowflakeId(99999)).await.unwrap();
        assert!(wallets.is_empty());
    }

    #[tokio::test]
    async fn find_or_create_creates_new() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let w = find_or_create(&pool, user.id, "CNY").await.unwrap();
        assert_eq!(w.currency, "CNY");
        assert_eq!(w.balance, 0);
    }

    #[tokio::test]
    async fn find_or_create_returns_existing() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let w1 = find_or_create(&pool, user.id, "CNY").await.unwrap();
        let w2 = find_or_create(&pool, user.id, "CNY").await.unwrap();
        assert_eq!(w1.id, w2.id);
    }

    #[tokio::test]
    async fn find_all_wallets_paginated() {
        let pool = setup_pool().await;
        let user1 = insert_user(&pool).await;
        let user2 = insert_user(&pool).await;
        create(&pool, user1.id, "CNY").await.unwrap();
        create(&pool, user2.id, "CNY").await.unwrap();
        let (rows, total) = find_all_wallets(&pool, 1, 10, None).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn find_all_wallets_page_two_empty() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        create(&pool, user.id, "CNY").await.unwrap();
        let (rows, total) = find_all_wallets(&pool, 2, 10, None).await.unwrap();
        assert_eq!(total, 1);
        assert!(rows.is_empty());
    }
}

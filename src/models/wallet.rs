use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

define_enum!(
    WalletStatus {
        Active = "active",
        Frozen = "frozen",
    }
);

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Wallet {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub currency: String,
    pub balance: i64,
    pub version: i64,
    pub status: WalletStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_user_and_currency(
    pool: &crate::db::Pool,
    user_id: i64,
    currency: &str,
) -> AppResult<Option<Wallet>> {
    let sql = format!(
        "SELECT * FROM wallets WHERE user_id = {} AND currency = {}",
        ph(1),
        ph(2)
    );
    sqlx::query_as::<_, Wallet>(&sql)
        .bind(user_id)
        .bind(currency)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<Option<Wallet>> {
    let sql = format!("SELECT * FROM wallets WHERE id = {}", ph(1));
    sqlx::query_as::<_, Wallet>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<Vec<Wallet>> {
    let sql = format!("SELECT * FROM wallets WHERE user_id = {}", ph(1));
    sqlx::query_as::<_, Wallet>(&sql)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub async fn create(pool: &crate::db::Pool, user_id: i64, currency: &str) -> AppResult<Wallet> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO wallets (document_id, user_id, currency, created_at, updated_at) VALUES ({}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5)
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(user_id)
        .bind(currency)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

    let sql = format!("SELECT * FROM wallets WHERE document_id = {}", ph(1));
    sqlx::query_as::<_, Wallet>(&sql)
        .bind(&document_id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_or_create(
    pool: &crate::db::Pool,
    user_id: i64,
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
    let offset = (page - 1) * page_size;
    let (total,): (i64,) = if let Some(tid) = tenant_id {
        sqlx::query_as(&format!(
            "SELECT COUNT(*) as count FROM wallets WHERE tenant_id = {}",
            crate::db::dialect::ph(1)
        ))
        .bind(tid)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as("SELECT COUNT(*) as count FROM wallets")
            .fetch_one(pool)
            .await?
    };
    let rows = if let Some(tid) = tenant_id {
        let sql = format!(
            "SELECT * FROM wallets WHERE tenant_id = {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2),
            crate::db::dialect::ph(3)
        );
        sqlx::query_as::<_, Wallet>(&sql)
            .bind(tid)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?
    } else {
        let sql = format!(
            "SELECT * FROM wallets ORDER BY created_at DESC LIMIT {} OFFSET {}",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2)
        );
        sqlx::query_as::<_, Wallet>(&sql)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?
    };
    Ok((rows, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn insert_user(pool: &crate::db::Pool) -> crate::models::user::User {
        crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_document_id(),
                registered_via: crate::models::user::RegisteredVia::Email,
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
        assert_eq!(found.document_id, w.document_id);
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(find_by_id(&pool, 99999).await.unwrap().is_none());
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
        let wallets = find_by_user(&pool, 99999).await.unwrap();
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

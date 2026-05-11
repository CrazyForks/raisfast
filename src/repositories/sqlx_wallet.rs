use crate::errors::app_error::AppResult;
use crate::models::wallet::{self, Wallet};
use crate::models::wallet_transaction::{self, WalletTransaction};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxWalletRepository);

#[async_trait::async_trait]
pub trait WalletRepository: Send + Sync {
    async fn find_wallet_by_user_and_currency(
        &self,
        user_id: i64,
        currency: &str,
    ) -> AppResult<Option<Wallet>>;

    async fn find_wallet_by_id(&self, id: i64) -> AppResult<Option<Wallet>>;

    async fn find_wallets_by_user(&self, user_id: i64) -> AppResult<Vec<Wallet>>;

    async fn create_wallet(&self, user_id: i64, currency: &str) -> AppResult<Wallet>;

    async fn find_or_create_wallet(&self, user_id: i64, currency: &str) -> AppResult<Wallet>;

    async fn find_all_wallets(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Wallet>, i64)>;

    async fn find_transactions_by_wallet(
        &self,
        wallet_id: i64,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WalletTransaction>, i64)>;

    async fn find_transactions_by_user(
        &self,
        user_id: i64,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WalletTransaction>, i64)>;

    async fn find_all_transactions(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WalletTransaction>, i64)>;

    async fn find_tx_by_transaction_no(
        &self,
        transaction_no: &str,
    ) -> AppResult<Option<WalletTransaction>>;

    async fn find_tx_by_id(&self, id: i64) -> AppResult<Option<WalletTransaction>>;

    async fn find_tx_by_document_id(&self, document_id: &str) -> AppResult<Option<WalletTransaction>>;

    async fn has_reversal_for(&self, related_tx_id: i64) -> AppResult<bool>;

    async fn find_document_ids_by_ids(
        &self,
        ids: &[i64],
    ) -> AppResult<std::collections::HashMap<i64, String>>;
}

#[async_trait::async_trait]
impl WalletRepository for SqlxWalletRepository {
    async fn find_wallet_by_user_and_currency(
        &self,
        user_id: i64,
        currency: &str,
    ) -> AppResult<Option<Wallet>> {
        wallet::find_by_user_and_currency(&self.pool, user_id, currency).await
    }

    async fn find_wallet_by_id(&self, id: i64) -> AppResult<Option<Wallet>> {
        wallet::find_by_id(&self.pool, id).await
    }

    async fn find_wallets_by_user(&self, user_id: i64) -> AppResult<Vec<Wallet>> {
        wallet::find_by_user(&self.pool, user_id).await
    }

    async fn create_wallet(&self, user_id: i64, currency: &str) -> AppResult<Wallet> {
        wallet::create(&self.pool, user_id, currency).await
    }

    async fn find_or_create_wallet(&self, user_id: i64, currency: &str) -> AppResult<Wallet> {
        wallet::find_or_create(&self.pool, user_id, currency).await
    }

    async fn find_all_wallets(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Wallet>, i64)> {
        wallet::find_all_wallets(&self.pool, page, page_size).await
    }

    async fn find_transactions_by_wallet(
        &self,
        wallet_id: i64,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
        wallet_transaction::find_transactions_by_wallet(&self.pool, wallet_id, page, page_size).await
    }

    async fn find_transactions_by_user(
        &self,
        user_id: i64,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
        wallet_transaction::find_transactions_by_user(&self.pool, user_id, page, page_size).await
    }

    async fn find_all_transactions(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
        wallet_transaction::find_all_transactions(&self.pool, page, page_size).await
    }

    async fn find_tx_by_transaction_no(
        &self,
        transaction_no: &str,
    ) -> AppResult<Option<WalletTransaction>> {
        wallet_transaction::find_tx_by_transaction_no(&self.pool, transaction_no).await
    }

    async fn find_tx_by_id(&self, id: i64) -> AppResult<Option<WalletTransaction>> {
        wallet_transaction::find_tx_by_id(&self.pool, id).await
    }

    async fn find_tx_by_document_id(&self, document_id: &str) -> AppResult<Option<WalletTransaction>> {
        wallet_transaction::find_tx_by_document_id(&self.pool, document_id).await
    }

    async fn has_reversal_for(&self, related_tx_id: i64) -> AppResult<bool> {
        wallet_transaction::has_reversal_for(&self.pool, related_tx_id).await
    }

    async fn find_document_ids_by_ids(
        &self,
        ids: &[i64],
    ) -> AppResult<std::collections::HashMap<i64, String>> {
        wallet_transaction::find_document_ids_by_ids(&self.pool, ids).await
    }
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

    fn setup_repo(pool: &crate::db::Pool) -> SqlxWalletRepository {
        SqlxWalletRepository::new(pool.clone())
    }

    async fn insert_user(pool: &crate::db::Pool) -> crate::models::user::User {
        crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_document_id(),
                registered_via: "test".to_string(),
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_and_find_wallet() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        let w = repo.create_wallet(user.id, "CNY").await.unwrap();
        let found = repo.find_wallet_by_id(w.id).await.unwrap().unwrap();
        assert_eq!(found.currency, "CNY");
    }

    #[tokio::test]
    async fn find_wallet_by_user_and_currency() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        repo.create_wallet(user.id, "CNY").await.unwrap();
        let found = repo
            .find_wallet_by_user_and_currency(user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.user_id, user.id);
    }

    #[tokio::test]
    async fn find_wallets_by_user_multiple() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        repo.create_wallet(user.id, "CNY").await.unwrap();
        repo.create_wallet(user.id, "USD").await.unwrap();
        let wallets = repo.find_wallets_by_user(user.id).await.unwrap();
        assert_eq!(wallets.len(), 2);
    }

    #[tokio::test]
    async fn find_or_create_creates_then_returns() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        let w1 = repo.find_or_create_wallet(user.id, "CNY").await.unwrap();
        let w2 = repo.find_or_create_wallet(user.id, "CNY").await.unwrap();
        assert_eq!(w1.id, w2.id);
    }

    #[tokio::test]
    async fn find_all_wallets_paginated() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        repo.create_wallet(user.id, "CNY").await.unwrap();
        repo.create_wallet(user.id, "USD").await.unwrap();
        let (rows, total) = repo.find_all_wallets(1, 10).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn find_transactions_by_wallet() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        let w = repo.create_wallet(user.id, "CNY").await.unwrap();

        let (doc_id, now) = crate::utils::id::new_document_id_and_timestamp();
        let tx_no = format!("TX_{doc_id}");
        sqlx::query(&format!(
            "INSERT INTO wallet_transactions (document_id, wallet_id, user_id, entry_type, amount, balance_after, tx_type, currency, transaction_no, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            crate::db::dialect::ph(1), crate::db::dialect::ph(2), crate::db::dialect::ph(3),
            crate::db::dialect::ph(4), crate::db::dialect::ph(5), crate::db::dialect::ph(6),
            crate::db::dialect::ph(7), crate::db::dialect::ph(8), crate::db::dialect::ph(9),
            crate::db::dialect::ph(10),
        ))
        .bind(&doc_id)
        .bind(w.id)
        .bind(user.id)
        .bind("credit")
        .bind(500_i64)
        .bind(500_i64)
        .bind("recharge")
        .bind("CNY")
        .bind(&tx_no)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let (rows, total) = repo.find_transactions_by_wallet(w.id, 1, 10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].amount, 500);
    }

    #[tokio::test]
    async fn find_tx_by_transaction_no() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        let w = repo.create_wallet(user.id, "CNY").await.unwrap();

        let (doc_id, now) = crate::utils::id::new_document_id_and_timestamp();
        let tx_no = format!("TX_{doc_id}");
        sqlx::query(&format!(
            "INSERT INTO wallet_transactions (document_id, wallet_id, user_id, entry_type, amount, balance_after, tx_type, currency, transaction_no, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            crate::db::dialect::ph(1), crate::db::dialect::ph(2), crate::db::dialect::ph(3),
            crate::db::dialect::ph(4), crate::db::dialect::ph(5), crate::db::dialect::ph(6),
            crate::db::dialect::ph(7), crate::db::dialect::ph(8), crate::db::dialect::ph(9),
            crate::db::dialect::ph(10),
        ))
        .bind(&doc_id)
        .bind(w.id)
        .bind(user.id)
        .bind("credit")
        .bind(500_i64)
        .bind(500_i64)
        .bind("recharge")
        .bind("CNY")
        .bind(&tx_no)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let found = repo.find_tx_by_transaction_no(&tx_no).await.unwrap().unwrap();
        assert_eq!(found.amount, 500);
        assert!(repo.find_tx_by_transaction_no("nonexistent").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn has_reversal_for_false() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        assert!(!repo.has_reversal_for(99999).await.unwrap());
    }
}

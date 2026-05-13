use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

define_enum!(
    WalletEntryType {
        Credit = "credit",
        Debit = "debit",
    }
);

define_enum!(
    WalletTxType {
        Recharge = "recharge",
        Payment = "payment",
        Refund = "refund",
        TransferOut = "transfer_out",
        TransferIn = "transfer_in",
    }
);

define_enum!(
    WalletReferenceType {
        Admin = "admin",
        Checkin = "checkin",
        OrderReward = "order_reward",
        ApiUsage = "api_usage",
        PointsMall = "points_mall",
        Order = "order",
        Expiry = "expiry",
        Payment = "payment",
        PaymentRefund = "payment_refund",
    }
);

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct WalletTransaction {
    pub id: i64,
    pub document_id: String,
    pub wallet_id: i64,
    pub user_id: i64,
    pub entry_type: WalletEntryType,
    pub amount: i64,
    pub balance_after: i64,
    pub tx_type: WalletTxType,
    pub currency: String,
    pub transaction_no: String,
    pub related_tx_id: Option<i64>,
    pub reference_type: Option<WalletReferenceType>,
    pub reference_id: Option<String>,
    pub counterparty_wallet_id: Option<i64>,
    pub metadata: Option<String>,
    pub created_at: Timestamp,
}

pub async fn find_transactions_by_wallet(
    pool: &crate::db::Pool,
    wallet_id: i64,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WalletTransaction>, i64)> {
    let offset = (page - 1) * page_size;
    let count_sql = format!(
        "SELECT COUNT(*) as count FROM wallet_transactions WHERE wallet_id = {}",
        ph(1)
    );
    let (total,): (i64,) = sqlx::query_as(&count_sql)
        .bind(wallet_id)
        .fetch_one(pool)
        .await?;

    let sql = format!(
        "SELECT * FROM wallet_transactions WHERE wallet_id = {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        ph(1),
        ph(2),
        ph(3)
    );
    let rows = sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(wallet_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok((rows, total))
}

pub async fn find_transactions_by_user(
    pool: &crate::db::Pool,
    user_id: i64,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WalletTransaction>, i64)> {
    let offset = (page - 1) * page_size;
    let count_sql = format!(
        "SELECT COUNT(*) as count FROM wallet_transactions WHERE user_id = {}",
        ph(1)
    );
    let (total,): (i64,) = sqlx::query_as(&count_sql)
        .bind(user_id)
        .fetch_one(pool)
        .await?;

    let sql = format!(
        "SELECT * FROM wallet_transactions WHERE user_id = {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        ph(1),
        ph(2),
        ph(3)
    );
    let rows = sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(user_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok((rows, total))
}

pub async fn find_all_transactions(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WalletTransaction>, i64)> {
    let offset = (page - 1) * page_size;
    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) as count FROM wallet_transactions")
        .fetch_one(pool)
        .await?;
    let sql = format!(
        "SELECT * FROM wallet_transactions ORDER BY created_at DESC LIMIT {} OFFSET {}",
        ph(1),
        ph(2)
    );
    let rows = sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok((rows, total))
}

pub async fn find_tx_by_transaction_no(
    pool: &crate::db::Pool,
    transaction_no: &str,
) -> AppResult<Option<WalletTransaction>> {
    let sql = format!(
        "SELECT * FROM wallet_transactions WHERE transaction_no = {}",
        ph(1)
    );
    sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(transaction_no)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_tx_by_id(
    pool: &crate::db::Pool,
    id: i64,
) -> AppResult<Option<WalletTransaction>> {
    let sql = format!("SELECT * FROM wallet_transactions WHERE id = {}", ph(1));
    sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_tx_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
) -> AppResult<Option<WalletTransaction>> {
    let sql = format!(
        "SELECT * FROM wallet_transactions WHERE document_id = {}",
        ph(1)
    );
    sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn has_reversal_for(pool: &crate::db::Pool, related_tx_id: i64) -> AppResult<bool> {
    let sql = format!(
        "SELECT COUNT(*) as count FROM wallet_transactions WHERE related_tx_id = {} AND tx_type = {}",
        ph(1),
        ph(2)
    );
    let (count,): (i64,) = sqlx::query_as(&sql)
        .bind(related_tx_id)
        .bind(WalletTxType::Refund)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

pub async fn find_document_ids_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
) -> AppResult<std::collections::HashMap<i64, String>> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let placeholders: Vec<String> = ids.iter().enumerate().map(|(i, _)| ph(i + 1)).collect();
    let sql = format!(
        "SELECT id, document_id FROM wallet_transactions WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, (i64, String)>(&sql);
    for &id in ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows.into_iter().collect())
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

    async fn seed_wallet_and_tx(
        pool: &crate::db::Pool,
    ) -> (
        crate::models::user::User,
        crate::models::wallet::Wallet,
        WalletTransaction,
    ) {
        let user = insert_user(pool).await;
        let w = crate::models::wallet::create(pool, user.id, "CNY")
            .await
            .unwrap();

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
        .bind(WalletEntryType::Credit)
        .bind(1000_i64)
        .bind(1000_i64)
        .bind(WalletTxType::Recharge)
        .bind("CNY")
        .bind(&tx_no)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();

        let tx = find_tx_by_transaction_no(pool, &tx_no)
            .await
            .unwrap()
            .unwrap();
        (user, w, tx)
    }

    #[tokio::test]
    async fn find_tx_by_transaction_no_found() {
        let pool = setup_pool().await;
        let (_, _, tx) = seed_wallet_and_tx(&pool).await;
        let found = find_tx_by_transaction_no(&pool, &tx.transaction_no)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.amount, 1000);
        assert_eq!(found.entry_type, WalletEntryType::Credit);
    }

    #[tokio::test]
    async fn find_tx_by_transaction_no_not_found() {
        let pool = setup_pool().await;
        assert!(
            find_tx_by_transaction_no(&pool, "nonexistent")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_tx_by_id_found() {
        let pool = setup_pool().await;
        let (_, _, tx) = seed_wallet_and_tx(&pool).await;
        let found = find_tx_by_id(&pool, tx.id).await.unwrap().unwrap();
        assert_eq!(found.transaction_no, tx.transaction_no);
    }

    #[tokio::test]
    async fn find_tx_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(find_tx_by_id(&pool, 99999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_transactions_by_wallet_found() {
        let pool = setup_pool().await;
        let (_, w, _) = seed_wallet_and_tx(&pool).await;
        let (rows, total) = find_transactions_by_wallet(&pool, w.id, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry_type, WalletEntryType::Credit);
    }

    #[tokio::test]
    async fn find_transactions_by_wallet_empty() {
        let pool = setup_pool().await;
        let (rows, total) = find_transactions_by_wallet(&pool, 99999, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn find_transactions_by_user_found() {
        let pool = setup_pool().await;
        let (user, _, _) = seed_wallet_and_tx(&pool).await;
        let (rows, total) = find_transactions_by_user(&pool, user.id, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn has_reversal_for_false() {
        let pool = setup_pool().await;
        let (_, _, tx) = seed_wallet_and_tx(&pool).await;
        assert!(!has_reversal_for(&pool, tx.id).await.unwrap());
    }

    #[tokio::test]
    async fn has_reversal_for_true() {
        let pool = setup_pool().await;
        let (_, _, tx) = seed_wallet_and_tx(&pool).await;

        let (rev_doc_id, rev_now) = crate::utils::id::new_document_id_and_timestamp();
        let rev_no = format!("REV_{rev_doc_id}");
        sqlx::query(&format!(
            "INSERT INTO wallet_transactions (document_id, wallet_id, user_id, entry_type, amount, balance_after, tx_type, currency, transaction_no, related_tx_id, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            crate::db::dialect::ph(1), crate::db::dialect::ph(2), crate::db::dialect::ph(3),
            crate::db::dialect::ph(4), crate::db::dialect::ph(5), crate::db::dialect::ph(6),
            crate::db::dialect::ph(7), crate::db::dialect::ph(8), crate::db::dialect::ph(9),
            crate::db::dialect::ph(10), crate::db::dialect::ph(11),
        ))
        .bind(&rev_doc_id)
        .bind(tx.wallet_id)
        .bind(tx.user_id)
        .bind(WalletEntryType::Debit)
        .bind(1000_i64)
        .bind(0_i64)
        .bind(WalletTxType::Refund)
        .bind("CNY")
        .bind(&rev_no)
        .bind(tx.id)
        .bind(rev_now)
        .execute(&pool)
        .await
        .unwrap();

        assert!(has_reversal_for(&pool, tx.id).await.unwrap());
    }
}

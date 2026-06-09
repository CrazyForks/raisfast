use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
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
    pub id: SnowflakeId,
    pub wallet_id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub entry_type: WalletEntryType,
    pub amount: i64,
    pub balance_after: i64,
    pub tx_type: WalletTxType,
    pub currency: String,
    pub transaction_no: String,
    pub related_tx_id: Option<SnowflakeId>,
    pub reference_type: Option<WalletReferenceType>,
    pub reference_id: Option<String>,
    pub counterparty_wallet_id: Option<SnowflakeId>,
    pub metadata: Option<String>,
    pub created_at: Timestamp,
}

pub async fn find_transactions_by_wallet(
    pool: &crate::db::Pool,
    wallet_id: SnowflakeId,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WalletTransaction>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, WalletTransaction,
        table: "wallet_transactions",
        where: ("wallet_id", wallet_id),
        order_by: "created_at DESC",
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_transactions_by_user(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WalletTransaction>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, WalletTransaction,
        table: "wallet_transactions",
        where: ("user_id", user_id),
        order_by: "created_at DESC",
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_all_transactions(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<WalletTransaction>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, WalletTransaction,
        table: "wallet_transactions",
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_tx_by_transaction_no(
    pool: &crate::db::Pool,
    transaction_no: &str,
) -> AppResult<Option<WalletTransaction>> {
    let result: Option<WalletTransaction> = raisfast_derive::crud_find!(pool, "wallet_transactions", WalletTransaction, where: ("transaction_no", transaction_no))?;
    Ok(result)
}

pub async fn find_tx_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<Option<WalletTransaction>> {
    let result: Option<WalletTransaction> = raisfast_derive::crud_find!(pool, "wallet_transactions", WalletTransaction, where: ("id", id))?;
    Ok(result)
}

pub async fn has_reversal_for(
    pool: &crate::db::Pool,
    related_tx_id: SnowflakeId,
) -> AppResult<bool> {
    Ok(raisfast_derive::crud_exists!(
        pool, "wallet_transactions", where: AND(("related_tx_id", related_tx_id), ("tx_type", WalletTxType::Refund))
    )?)
}

pub async fn tx_find_by_id(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
) -> AppResult<Option<WalletTransaction>> {
    Ok(
        raisfast_derive::crud_find!(tx, "wallet_transactions", WalletTransaction, where: ("id", id))?,
    )
}

pub async fn tx_find_by_transaction_no(
    tx: &mut crate::db::pool::DbConnection,
    transaction_no: &str,
) -> AppResult<Option<WalletTransaction>> {
    Ok(
        raisfast_derive::crud_find!(tx, "wallet_transactions", WalletTransaction, where: ("transaction_no", transaction_no))?,
    )
}

pub async fn tx_has_reversal_for(
    tx: &mut crate::db::pool::DbConnection,
    related_tx_id: SnowflakeId,
) -> AppResult<bool> {
    Ok(raisfast_derive::crud_exists!(
        tx, "wallet_transactions", where: AND(("related_tx_id", related_tx_id), ("tx_type", WalletTxType::Refund))
    )?)
}

#[allow(clippy::too_many_arguments)]
pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    wallet_id: SnowflakeId,
    user_id: SnowflakeId,
    entry_type: WalletEntryType,
    amount: i64,
    balance_after: i64,
    tx_type: WalletTxType,
    currency: &str,
    transaction_no: &str,
    related_tx_id: Option<SnowflakeId>,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<String>,
    counterparty_wallet_id: Option<SnowflakeId>,
    metadata: Option<String>,
) -> AppResult<WalletTransaction> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        &mut *tx, "wallet_transactions",
        ["id" => id, "wallet_id" => wallet_id, "user_id" => user_id, "entry_type" => entry_type, "amount" => amount, "balance_after" => balance_after, "tx_type" => tx_type, "currency" => currency, "transaction_no" => transaction_no, "related_tx_id" => related_tx_id, "reference_type" => reference_type, "reference_id" => reference_id, "counterparty_wallet_id" => counterparty_wallet_id, "metadata" => metadata, "created_at" => now]
    )?;
    Ok(
        raisfast_derive::crud_find_one!(&mut *tx, "wallet_transactions", WalletTransaction, where: ("id", id))?,
    )
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
                username: crate::utils::id::new_id().to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
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

        let (tx_id, now) = (
            crate::utils::id::new_snowflake_id(),
            crate::utils::tz::now_utc(),
        );
        let tx_no = format!("TX_{tx_id}");
        raisfast_derive::crud_insert!(pool, "wallet_transactions", [
            "id" => tx_id,
            "wallet_id" => w.id,
            "user_id" => user.id,
            "entry_type" => WalletEntryType::Credit,
            "amount" => 1000_i64,
            "balance_after" => 1000_i64,
            "tx_type" => WalletTxType::Recharge,
            "currency" => "CNY",
            "transaction_no" => &tx_no,
            "created_at" => now
        ])
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
        assert!(
            find_tx_by_id(&pool, SnowflakeId(99999))
                .await
                .unwrap()
                .is_none()
        );
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
        let (rows, total) = find_transactions_by_wallet(&pool, SnowflakeId(99999), 1, 10)
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

        let (rev_id, rev_now) = (
            crate::utils::id::new_snowflake_id(),
            crate::utils::tz::now_utc(),
        );
        let rev_no = format!("REV_{rev_id}");
        raisfast_derive::crud_insert!(&pool, "wallet_transactions", [
            "id" => rev_id,
            "wallet_id" => tx.wallet_id,
            "user_id" => tx.user_id,
            "entry_type" => WalletEntryType::Debit,
            "amount" => 1000_i64,
            "balance_after" => 0_i64,
            "tx_type" => WalletTxType::Refund,
            "currency" => "CNY",
            "transaction_no" => &rev_no,
            "related_tx_id" => tx.id,
            "created_at" => rev_now
        ])
        .unwrap();

        assert!(has_reversal_for(&pool, tx.id).await.unwrap());
    }
}

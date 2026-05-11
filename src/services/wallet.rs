use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::models::wallet;
use crate::models::wallet_transaction::WalletTransaction;
use crate::repositories::WalletRepository;

#[cfg(feature = "db-sqlite")]
async fn tx_find_wallet_by_id(
    tx: &mut sqlx::SqliteConnection,
    id: i64,
) -> AppResult<Option<wallet::Wallet>> {
    let sql = "SELECT * FROM wallets WHERE id = ?1";
    sqlx::query_as::<_, wallet::Wallet>(sql)
        .bind(id)
        .fetch_optional(tx)
        .await
        .map_err(Into::into)
}

#[cfg(feature = "db-sqlite")]
async fn tx_find_or_create(
    tx: &mut sqlx::SqliteConnection,
    user_id: i64,
    currency: &str,
) -> AppResult<wallet::Wallet> {
    let sql = "SELECT * FROM wallets WHERE user_id = ?1 AND currency = ?2";
    if let Some(w) = sqlx::query_as::<_, wallet::Wallet>(sql)
        .bind(user_id)
        .bind(currency)
        .fetch_optional(&mut *tx)
        .await?
    {
        return Ok(w);
    }
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = "INSERT INTO wallets (document_id, user_id, currency, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)";
    sqlx::query(sql)
        .bind(&document_id)
        .bind(user_id)
        .bind(currency)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    let sql = "SELECT * FROM wallets WHERE document_id = ?1";
    sqlx::query_as::<_, wallet::Wallet>(sql)
        .bind(&document_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(Into::into)
}

#[cfg(feature = "db-sqlite")]
async fn tx_find_tx_by_id(
    tx: &mut sqlx::SqliteConnection,
    id: i64,
) -> AppResult<Option<WalletTransaction>> {
    let sql = "SELECT * FROM wallet_transactions WHERE id = ?1";
    sqlx::query_as::<_, WalletTransaction>(sql)
        .bind(id)
        .fetch_optional(tx)
        .await
        .map_err(Into::into)
}

#[cfg(feature = "db-sqlite")]
async fn tx_has_reversal_for(
    tx: &mut sqlx::SqliteConnection,
    related_tx_id: i64,
) -> AppResult<bool> {
    let sql = "SELECT COUNT(*) as count FROM wallet_transactions WHERE related_tx_id = ?1 AND tx_type = 'refund'";
    let (count,): (i64,) = sqlx::query_as(sql)
        .bind(related_tx_id)
        .fetch_one(tx)
        .await?;
    Ok(count > 0)
}

#[allow(clippy::too_many_arguments)]
pub async fn credit_wallet(
    repo: &dyn WalletRepository,
    pool: &crate::db::Pool,
    user_id: i64,
    currency: &str,
    amount: i64,
    tx_type: &str,
    transaction_no: &str,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    if amount <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive".into()));
    }

    if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
        return Ok(existing);
    }

    crate::in_transaction!(pool, tx, {
        if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
            return Ok(existing);
        }

        let w = tx_find_or_create(&mut tx, user_id, currency).await?;
        if w.status != "active" {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        let sql = format!(
            "UPDATE wallets SET balance = balance + {}, version = version + 1, updated_at = {} WHERE id = {} AND version = {}",
            ph(1), ph(2), ph(3), ph(4)
        );
        let affected = sqlx::query(&sql)
            .bind(amount)
            .bind(crate::utils::tz::now_str())
            .bind(w.id)
            .bind(w.version)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::Conflict("concurrent_wallet_update".into()));
        }

        let updated = tx_find_wallet_by_id(&mut tx, w.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found after update")))?;

        insert_tx(
            &mut tx, updated.id, user_id, "credit", amount, updated.balance,
            tx_type, currency, transaction_no, None, reference_type, reference_id, None, metadata,
        ).await
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn debit_wallet(
    repo: &dyn WalletRepository,
    pool: &crate::db::Pool,
    user_id: i64,
    currency: &str,
    amount: i64,
    tx_type: &str,
    transaction_no: &str,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    if amount <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive".into()));
    }

    if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
        return Ok(existing);
    }

    crate::in_transaction!(pool, tx, {
        if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
            return Ok(existing);
        }

        let w = tx_find_or_create(&mut tx, user_id, currency).await?;
        if w.status != "active" {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        let sql = format!(
            "UPDATE wallets SET balance = balance - {}, version = version + 1, updated_at = {} WHERE id = {} AND balance >= {} AND version = {}",
            ph(1), ph(2), ph(3), ph(4), ph(5)
        );
        let affected = sqlx::query(&sql)
            .bind(amount)
            .bind(crate::utils::tz::now_str())
            .bind(w.id)
            .bind(amount)
            .bind(w.version)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::BadRequest("insufficient_balance_or_concurrent_update".into()));
        }

        let updated = tx_find_wallet_by_id(&mut tx, w.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found after update")))?;

        insert_tx(
            &mut tx, updated.id, user_id, "debit", amount, updated.balance,
            tx_type, currency, transaction_no, None, reference_type, reference_id, None, metadata,
        ).await
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn transfer(
    repo: &dyn WalletRepository,
    pool: &crate::db::Pool,
    from_user_id: i64,
    to_user_id: i64,
    currency: &str,
    amount: i64,
    transaction_no: &str,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<(WalletTransaction, WalletTransaction)> {
    if amount <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive".into()));
    }

    if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
        let pair_no = format!("{transaction_no}_in");
        let incoming = repo.find_tx_by_transaction_no(&pair_no).await?;
        let incoming = incoming.ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("transfer pair incomplete"))
        })?;
        return Ok((existing, incoming));
    }

    crate::in_transaction!(pool, tx, {
        if repo.find_tx_by_transaction_no(transaction_no).await?.is_some() {
            return Err(AppError::Conflict("duplicate_transaction".into()));
        }

        let from_wallet = tx_find_or_create(&mut tx, from_user_id, currency).await?;
        let to_wallet = tx_find_or_create(&mut tx, to_user_id, currency).await?;

        if from_wallet.id == to_wallet.id {
            return Err(AppError::BadRequest("cannot_transfer_to_same_wallet".into()));
        }

        if from_wallet.status != "active" || to_wallet.status != "active" {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        let sql = format!(
            "UPDATE wallets SET balance = balance - {}, version = version + 1, updated_at = {} WHERE id = {} AND balance >= {} AND version = {}",
            ph(1), ph(2), ph(3), ph(4), ph(5)
        );
        let affected = sqlx::query(&sql)
            .bind(amount)
            .bind(crate::utils::tz::now_str())
            .bind(from_wallet.id)
            .bind(amount)
            .bind(from_wallet.version)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::BadRequest("insufficient_balance_or_concurrent_update".into()));
        }

        let sql = format!(
            "UPDATE wallets SET balance = balance + {}, version = version + 1, updated_at = {} WHERE id = {} AND version = {}",
            ph(1), ph(2), ph(3), ph(4)
        );
        let affected = sqlx::query(&sql)
            .bind(amount)
            .bind(crate::utils::tz::now_str())
            .bind(to_wallet.id)
            .bind(to_wallet.version)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::Conflict("concurrent_wallet_update".into()));
        }

        let updated_from = tx_find_wallet_by_id(&mut tx, from_wallet.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found")))?;
        let updated_to = tx_find_wallet_by_id(&mut tx, to_wallet.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found")))?;

        let out_tx = insert_tx(
            &mut tx, updated_from.id, from_user_id, "debit", amount, updated_from.balance,
            "transfer_out", currency, transaction_no, None, reference_type, reference_id,
            Some(updated_to.id), metadata,
        ).await?;

        let in_no = format!("{transaction_no}_in");
        let in_tx = insert_tx(
            &mut tx, updated_to.id, to_user_id, "credit", amount, updated_to.balance,
            "transfer_in", currency, &in_no, None, reference_type, reference_id,
            Some(updated_from.id), metadata,
        ).await?;

        Ok((out_tx, in_tx))
    })
}

pub async fn reverse_transaction(
    repo: &dyn WalletRepository,
    pool: &crate::db::Pool,
    original_tx_id: i64,
    transaction_no: &str,
) -> AppResult<WalletTransaction> {
    if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
        return Ok(existing);
    }

    crate::in_transaction!(pool, tx, {
        if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
            return Ok(existing);
        }

        let original = tx_find_tx_by_id(&mut tx, original_tx_id)
            .await?
            .ok_or_else(|| AppError::not_found("transaction"))?;

        if original.tx_type == "refund" {
            return Err(AppError::BadRequest("cannot_reverse_reversal".into()));
        }

        if tx_has_reversal_for(&mut tx, original_tx_id).await? {
            return Err(AppError::BadRequest("already_reversed".into()));
        }

        let w = tx_find_wallet_by_id(&mut tx, original.wallet_id)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))?;

        let (entry_type, amount_delta) = match original.entry_type.as_str() {
            "credit" => ("debit", -original.amount),
            "debit" => ("credit", original.amount),
            _ => return Err(AppError::BadRequest("invalid_entry_type".into())),
        };

        if amount_delta > 0 {
            let sql = format!(
                "UPDATE wallets SET balance = balance + {}, version = version + 1, updated_at = {} WHERE id = {} AND version = {}",
                ph(1), ph(2), ph(3), ph(4)
            );
            sqlx::query(&sql)
                .bind(amount_delta)
                .bind(crate::utils::tz::now_str())
                .bind(w.id)
                .bind(w.version)
                .execute(&mut *tx)
                .await?;
        } else {
            let abs_delta = -amount_delta;
            let sql = format!(
                "UPDATE wallets SET balance = balance - {}, version = version + 1, updated_at = {} WHERE id = {} AND balance >= {} AND version = {}",
                ph(1), ph(2), ph(3), ph(4), ph(5)
            );
            let affected = sqlx::query(&sql)
                .bind(abs_delta)
                .bind(crate::utils::tz::now_str())
                .bind(w.id)
                .bind(abs_delta)
                .bind(w.version)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if affected == 0 {
                return Err(AppError::BadRequest(
                    "insufficient_balance_for_reversal".into(),
                ));
            }
        }

        let updated = tx_find_wallet_by_id(&mut tx, w.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found")))?;

        insert_tx(
            &mut tx, updated.id, original.user_id, entry_type, original.amount, updated.balance,
            "refund", &original.currency, transaction_no, Some(original_tx_id),
            original.reference_type.as_deref(), original.reference_id.as_deref(), None,
            Some(&serde_json::json!({"reversal": true}).to_string()),
        ).await
    })
}

#[cfg(feature = "db-sqlite")]
#[allow(clippy::too_many_arguments)]
async fn insert_tx(
    tx: &mut sqlx::SqliteConnection,
    wallet_id: i64,
    user_id: i64,
    entry_type: &str,
    amount: i64,
    balance_after: i64,
    tx_type: &str,
    currency: &str,
    transaction_no: &str,
    related_tx_id: Option<i64>,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
    counterparty_wallet_id: Option<i64>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO wallet_transactions (document_id, wallet_id, user_id, entry_type, amount, balance_after, tx_type, currency, transaction_no, related_tx_id, reference_type, reference_id, counterparty_wallet_id, metadata, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11), ph(12), ph(13), ph(14), ph(15)
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(wallet_id)
        .bind(user_id)
        .bind(entry_type)
        .bind(amount)
        .bind(balance_after)
        .bind(tx_type)
        .bind(currency)
        .bind(transaction_no)
        .bind(related_tx_id)
        .bind(reference_type)
        .bind(reference_id)
        .bind(counterparty_wallet_id)
        .bind(metadata)
        .bind(now)
        .execute(&mut *tx)
        .await?;

    let (row_id,): (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&mut *tx)
        .await?;

    Ok(WalletTransaction {
        id: row_id,
        document_id,
        wallet_id,
        user_id,
        entry_type: entry_type.to_string(),
        amount,
        balance_after,
        tx_type: tx_type.to_string(),
        currency: currency.to_string(),
        transaction_no: transaction_no.to_string(),
        related_tx_id,
        reference_type: reference_type.map(|s| s.to_string()),
        reference_id: reference_id.map(|s| s.to_string()),
        counterparty_wallet_id,
        metadata: metadata.map(|s| s.to_string()),
        created_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::app_error::AppError;
    use crate::repositories::sqlx_wallet::SqlxWalletRepository;

    async fn setup_pool() -> crate::db::Pool {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("raisfast_wallet_test_{id}.db"));
        let url = format!("sqlite:{}?mode=rwc", path.display());
        let pool = crate::db::Pool::connect(&url).await.unwrap();
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
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

    fn new_tx_no() -> String {
        format!("TX_{}", crate::utils::id::new_document_id())
    }

    // ── credit_wallet ──

    #[tokio::test]
    async fn credit_normal() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        let tx_no = new_tx_no();

        let tx = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &tx_no,
            Some("admin"), None, None,
        )
        .await
        .unwrap();

        assert_eq!(tx.entry_type, "credit");
        assert_eq!(tx.amount, 500);
        assert_eq!(tx.balance_after, 500);
        assert_eq!(tx.tx_type, "recharge");

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 500);
    }

    #[tokio::test]
    async fn credit_auto_creates_wallet() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        assert!(wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .is_none());

        credit_wallet(
            &repo, &pool, user.id, "CNY", 100, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 100);
    }

    #[tokio::test]
    async fn credit_idempotent() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        let tx_no = new_tx_no();

        let tx1 = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &tx_no,
            None, None, None,
        )
        .await
        .unwrap();

        let tx2 = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &tx_no,
            None, None, None,
        )
        .await
        .unwrap();

        assert_eq!(tx1.transaction_no, tx2.transaction_no);

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 500);
    }

    #[tokio::test]
    async fn credit_amount_zero_rejected() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let err = credit_wallet(
            &repo, &pool, user.id, "CNY", 0, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "amount_must_be_positive"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn credit_negative_amount_rejected() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let err = credit_wallet(
            &repo, &pool, user.id, "CNY", -100, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "amount_must_be_positive"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn credit_multiple_accumulates() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, user.id, "CNY", 300, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();
        credit_wallet(
            &repo, &pool, user.id, "CNY", 700, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 1000);
    }

    // ── debit_wallet ──

    #[tokio::test]
    async fn debit_normal() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, user.id, "CNY", 1000, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let tx = debit_wallet(
            &repo, &pool, user.id, "CNY", 400, "payment", &new_tx_no(),
            Some("order"), None, None,
        )
        .await
        .unwrap();

        assert_eq!(tx.entry_type, "debit");
        assert_eq!(tx.amount, 400);
        assert_eq!(tx.balance_after, 600);

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 600);
    }

    #[tokio::test]
    async fn debit_insufficient_balance() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, user.id, "CNY", 100, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let err = debit_wallet(
            &repo, &pool, user.id, "CNY", 200, "payment", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => {
                assert_eq!(msg, "insufficient_balance_or_concurrent_update");
            }
            _ => panic!("expected BadRequest"),
        }

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 100);
    }

    #[tokio::test]
    async fn debit_exact_balance() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let tx = debit_wallet(
            &repo, &pool, user.id, "CNY", 500, "payment", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        assert_eq!(tx.balance_after, 0);
    }

    #[tokio::test]
    async fn debit_idempotent() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;
        let tx_no = new_tx_no();

        credit_wallet(
            &repo, &pool, user.id, "CNY", 1000, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let tx1 = debit_wallet(
            &repo, &pool, user.id, "CNY", 300, "payment", &tx_no,
            None, None, None,
        )
        .await
        .unwrap();

        let tx2 = debit_wallet(
            &repo, &pool, user.id, "CNY", 300, "payment", &tx_no,
            None, None, None,
        )
        .await
        .unwrap();

        assert_eq!(tx1.transaction_no, tx2.transaction_no);

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 700);
    }

    #[tokio::test]
    async fn debit_amount_must_be_positive() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let err = debit_wallet(
            &repo, &pool, user.id, "CNY", 0, "payment", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "amount_must_be_positive"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn debit_no_wallet_auto_creates_then_fails_insufficient() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let err = debit_wallet(
            &repo, &pool, user.id, "CNY", 100, "payment", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => {
                assert_eq!(msg, "insufficient_balance_or_concurrent_update");
            }
            _ => panic!("expected BadRequest"),
        }
    }

    // ── transfer ──

    #[tokio::test]
    async fn transfer_normal() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let from_user = insert_user(&pool).await;
        let to_user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, from_user.id, "CNY", 1000, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (out_tx, in_tx) = transfer(
            &repo, &pool, from_user.id, to_user.id, "CNY", 300,
            &tx_no, None, None, None,
        )
        .await
        .unwrap();

        assert_eq!(out_tx.entry_type, "debit");
        assert_eq!(out_tx.tx_type, "transfer_out");
        assert_eq!(out_tx.amount, 300);
        assert_eq!(out_tx.balance_after, 700);

        assert_eq!(in_tx.entry_type, "credit");
        assert_eq!(in_tx.tx_type, "transfer_in");
        assert_eq!(in_tx.amount, 300);
        assert_eq!(in_tx.balance_after, 300);

        let from_w = wallet::find_by_user_and_currency(&pool, from_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_w.balance, 700);

        let to_w = wallet::find_by_user_and_currency(&pool, to_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(to_w.balance, 300);
    }

    #[tokio::test]
    async fn transfer_insufficient_balance() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let from_user = insert_user(&pool).await;
        let to_user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, from_user.id, "CNY", 100, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let err = transfer(
            &repo, &pool, from_user.id, to_user.id, "CNY", 200,
            &new_tx_no(), None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => {
                assert_eq!(msg, "insufficient_balance_or_concurrent_update");
            }
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn transfer_to_self_rejected() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, user.id, "CNY", 1000, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let err = transfer(
            &repo, &pool, user.id, user.id, "CNY", 100,
            &new_tx_no(), None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "cannot_transfer_to_same_wallet"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn transfer_idempotent() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let from_user = insert_user(&pool).await;
        let to_user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, from_user.id, "CNY", 1000, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (out1, in1) = transfer(
            &repo, &pool, from_user.id, to_user.id, "CNY", 300,
            &tx_no, None, None, None,
        )
        .await
        .unwrap();

        let (out2, in2) = transfer(
            &repo, &pool, from_user.id, to_user.id, "CNY", 300,
            &tx_no, None, None, None,
        )
        .await
        .unwrap();

        assert_eq!(out1.transaction_no, out2.transaction_no);
        assert_eq!(in1.transaction_no, in2.transaction_no);

        let from_w = wallet::find_by_user_and_currency(&pool, from_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_w.balance, 700);
    }

    #[tokio::test]
    async fn transfer_amount_must_be_positive() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let from_user = insert_user(&pool).await;
        let to_user = insert_user(&pool).await;

        let err = transfer(
            &repo, &pool, from_user.id, to_user.id, "CNY", 0,
            &new_tx_no(), None, None, None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "amount_must_be_positive"),
            _ => panic!("expected BadRequest"),
        }
    }

    // ── reverse_transaction ──

    #[tokio::test]
    async fn reverse_credit() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let original = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let rev_tx = reverse_transaction(
            &repo, &pool, original.id, &new_tx_no(),
        )
        .await
        .unwrap();

        assert_eq!(rev_tx.entry_type, "debit");
        assert_eq!(rev_tx.amount, 500);
        assert_eq!(rev_tx.balance_after, 0);
        assert_eq!(rev_tx.related_tx_id, Some(original.id));

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 0);
    }

    #[tokio::test]
    async fn reverse_debit() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, user.id, "CNY", 1000, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let original = debit_wallet(
            &repo, &pool, user.id, "CNY", 300, "payment", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let rev_tx = reverse_transaction(
            &repo, &pool, original.id, &new_tx_no(),
        )
        .await
        .unwrap();

        assert_eq!(rev_tx.entry_type, "credit");
        assert_eq!(rev_tx.amount, 300);
        assert_eq!(rev_tx.balance_after, 1000);

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 1000);
    }

    #[tokio::test]
    async fn reverse_idempotent() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let original = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let rev_no = new_tx_no();
        let rev1 = reverse_transaction(&repo, &pool, original.id, &rev_no).await.unwrap();
        let rev2 = reverse_transaction(&repo, &pool, original.id, &rev_no).await.unwrap();
        assert_eq!(rev1.transaction_no, rev2.transaction_no);
    }

    #[tokio::test]
    async fn reverse_cannot_reverse_reversal() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let original = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let rev = reverse_transaction(
            &repo, &pool, original.id, &new_tx_no(),
        )
        .await
        .unwrap();

        let err = reverse_transaction(&repo, &pool, rev.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "cannot_reverse_reversal"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn reverse_already_reversed_rejected() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        let original = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        reverse_transaction(&repo, &pool, original.id, &new_tx_no())
            .await
            .unwrap();

        let err = reverse_transaction(&repo, &pool, original.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "already_reversed"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn reverse_insufficient_balance_for_debit_reversal() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);
        let user = insert_user(&pool).await;

        credit_wallet(
            &repo, &pool, user.id, "CNY", 1000, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let credit_tx = credit_wallet(
            &repo, &pool, user.id, "CNY", 500, "recharge", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        debit_wallet(
            &repo, &pool, user.id, "CNY", 1400, "payment", &new_tx_no(),
            None, None, None,
        )
        .await
        .unwrap();

        let w = wallet::find_by_user_and_currency(&pool, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 100);

        let err = reverse_transaction(&repo, &pool, credit_tx.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => {
                assert_eq!(msg, "insufficient_balance_for_reversal");
            }
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn reverse_nonexistent_transaction() {
        let pool = setup_pool().await;
        let repo = setup_repo(&pool);

        let err = reverse_transaction(&repo, &pool, 99999, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::NotFound(_) => {}
            _ => panic!("expected NotFound"),
        }
    }
}

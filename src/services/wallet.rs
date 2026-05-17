use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::db::dialect::ph;
use crate::db::pool::DbConnection;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::models::currencies;
use crate::models::wallet;
use crate::models::wallet::WalletStatus;
use crate::models::wallet_transaction::WalletTransaction;
use crate::models::wallet_transaction::{WalletEntryType, WalletReferenceType, WalletTxType};

#[async_trait]
pub trait WalletService: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn credit(
        &self,
        user_id: i64,
        currency: &str,
        amount: i64,
        tx_type: WalletTxType,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<WalletTransaction>;

    #[allow(clippy::too_many_arguments)]
    async fn debit(
        &self,
        user_id: i64,
        currency: &str,
        amount: i64,
        tx_type: WalletTxType,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<WalletTransaction>;

    #[allow(clippy::too_many_arguments)]
    async fn transfer(
        &self,
        from_user_id: i64,
        to_user_id: i64,
        currency: &str,
        amount: i64,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<(WalletTransaction, WalletTransaction)>;

    async fn reverse_transaction(
        &self,
        original_tx_id: i64,
        transaction_no: &str,
    ) -> AppResult<WalletTransaction>;

    async fn tx_to_response(
        &self,
        tx: WalletTransaction,
    ) -> AppResult<crate::dto::WalletTransactionResponse>;

    async fn tx_list_to_response(
        &self,
        rows: Vec<WalletTransaction>,
    ) -> AppResult<Vec<crate::dto::WalletTransactionResponse>>;

    async fn find_user_int_id(&self, user_doc_id: &str, tenant_id: Option<&str>) -> AppResult<i64>;

    async fn list_wallets_by_user(
        &self,
        user_doc_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<crate::models::wallet::Wallet>>;

    async fn get_wallet_by_currency(
        &self,
        user_doc_id: &str,
        currency: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<crate::models::wallet::Wallet>;

    async fn list_transactions_by_wallet(
        &self,
        user_doc_id: &str,
        currency: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)>;

    async fn list_transactions_by_user(
        &self,
        user_doc_id: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)>;

    async fn list_all_wallets(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<crate::models::wallet::Wallet>, i64)>;

    async fn list_all_transactions(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)>;

    async fn find_tx_by_document_id(
        &self,
        tx_doc_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<WalletTransaction>;
}

pub struct WalletServiceImpl {
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
}

impl WalletServiceImpl {
    pub fn new(aspect_engine: Arc<AspectEngine>, pool: Arc<crate::db::Pool>) -> Self {
        Self {
            aspect_engine,
            pool,
        }
    }

    fn after_credited(&self, tx: &WalletTransaction) {
        self.aspect_engine.emit(Event::WalletCredited(tx.clone()));
    }

    fn after_debited(&self, tx: &WalletTransaction) {
        self.aspect_engine.emit(Event::WalletDebited(tx.clone()));
    }
}

async fn ensure_currency_active(tx: &mut DbConnection, currency: &str) -> AppResult<()> {
    currencies::find_by_code_tx(tx, currency)
        .await?
        .ok_or_else(|| AppError::BadRequest(format!("currency_not_active: {currency}")))?;
    Ok(())
}

// Re-export for test convenience
#[cfg(test)]
use crate::models::wallet_transaction::WalletEntryType as E;
#[cfg(test)]
use crate::models::wallet_transaction::WalletReferenceType as R;
#[cfg(test)]
use crate::models::wallet_transaction::WalletTxType as T;

async fn tx_find_wallet_by_id(tx: &mut DbConnection, id: i64) -> AppResult<Option<wallet::Wallet>> {
    check_schema!("wallets", "id");
    let sql = format!("SELECT * FROM wallets WHERE id = {}", ph(1));
    sqlx::query_as::<_, wallet::Wallet>(&sql)
        .bind(id)
        .fetch_optional(tx)
        .await
        .map_err(Into::into)
}

async fn tx_find_or_create(
    tx: &mut DbConnection,
    user_id: i64,
    currency: &str,
) -> AppResult<wallet::Wallet> {
    check_schema!(
        "wallets",
        "document_id",
        "user_id",
        "currency",
        "created_at",
        "updated_at"
    );
    let sql = format!(
        "SELECT * FROM wallets WHERE user_id = {} AND currency = {}",
        ph(1),
        ph(2)
    );
    if let Some(w) = sqlx::query_as::<_, wallet::Wallet>(&sql)
        .bind(user_id)
        .bind(currency)
        .fetch_optional(&mut *tx)
        .await?
    {
        return Ok(w);
    }
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO wallets (document_id, user_id, currency, created_at, updated_at) VALUES ({}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5)
    );
    let insert_result = sqlx::query(&sql)
        .bind(&document_id)
        .bind(user_id)
        .bind(currency)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;

    match insert_result {
        Ok(_) => {
            let sql = format!("SELECT * FROM wallets WHERE document_id = {}", ph(1));
            sqlx::query_as::<_, wallet::Wallet>(&sql)
                .bind(&document_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(Into::into)
        }
        Err(_) => {
            let sql = format!(
                "SELECT * FROM wallets WHERE user_id = {} AND currency = {}",
                ph(1),
                ph(2)
            );
            sqlx::query_as::<_, wallet::Wallet>(&sql)
                .bind(user_id)
                .bind(currency)
                .fetch_one(&mut *tx)
                .await
                .map_err(Into::into)
        }
    }
}

async fn tx_find_tx_by_id(tx: &mut DbConnection, id: i64) -> AppResult<Option<WalletTransaction>> {
    check_schema!("wallet_transactions", "id");
    let sql = format!("SELECT * FROM wallet_transactions WHERE id = {}", ph(1));
    sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(id)
        .fetch_optional(tx)
        .await
        .map_err(Into::into)
}

async fn tx_find_tx_by_transaction_no(
    tx: &mut DbConnection,
    transaction_no: &str,
) -> AppResult<Option<WalletTransaction>> {
    check_schema!("wallet_transactions", "transaction_no");
    let sql = format!(
        "SELECT * FROM wallet_transactions WHERE transaction_no = {}",
        ph(1)
    );
    sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(transaction_no)
        .fetch_optional(tx)
        .await
        .map_err(Into::into)
}

async fn tx_has_reversal_for(tx: &mut DbConnection, related_tx_id: i64) -> AppResult<bool> {
    check_schema!("wallet_transactions", "related_tx_id", "tx_type");
    let sql = format!(
        "SELECT COUNT(*) as count FROM wallet_transactions WHERE related_tx_id = {} AND tx_type = {}",
        ph(1),
        ph(2)
    );
    let (count,): (i64,) = sqlx::query_as(&sql)
        .bind(related_tx_id)
        .bind(WalletTxType::Refund)
        .fetch_one(tx)
        .await?;
    Ok(count > 0)
}

async fn apply_wallet_delta(
    tx: &mut DbConnection,
    wallet_id: i64,
    version: i64,
    delta: i64,
    current_balance: i64,
) -> AppResult<()> {
    check_schema!("wallets", "balance", "version", "updated_at", "id");
    if delta > 0 {
        let _ = current_balance
            .checked_add(delta)
            .ok_or_else(|| AppError::BadRequest("balance_overflow".into()))?;
        let sql = format!(
            "UPDATE wallets SET balance = balance + {}, version = version + 1, updated_at = {} WHERE id = {} AND version = {}",
            ph(1),
            ph(2),
            ph(3),
            ph(4)
        );
        let affected = sqlx::query(&sql)
            .bind(delta)
            .bind(crate::utils::tz::now_str())
            .bind(wallet_id)
            .bind(version)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::Conflict("concurrent_wallet_update".into()));
        }
    } else {
        let abs = -delta;
        let sql = format!(
            "UPDATE wallets SET balance = balance - {}, version = version + 1, updated_at = {} WHERE id = {} AND balance >= {} AND version = {}",
            ph(1),
            ph(2),
            ph(3),
            ph(4),
            ph(5)
        );
        let affected = sqlx::query(&sql)
            .bind(abs)
            .bind(crate::utils::tz::now_str())
            .bind(wallet_id)
            .bind(abs)
            .bind(version)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::BadRequest(
                "insufficient_balance_or_concurrent_update".into(),
            ));
        }
    }
    Ok(())
}

async fn reverse_single_tx(
    tx: &mut DbConnection,
    original: &WalletTransaction,
    reversal_tx_no: &str,
) -> AppResult<WalletTransaction> {
    let w = tx_find_wallet_by_id(tx, original.wallet_id)
        .await?
        .ok_or_else(|| AppError::not_found("wallet"))?;

    let delta = match original.entry_type {
        WalletEntryType::Credit => -original.amount,
        WalletEntryType::Debit => original.amount,
    };

    if delta > 0 {
        w.balance
            .checked_add(delta)
            .ok_or_else(|| AppError::BadRequest("balance_overflow".into()))?;
    } else if w.balance < -delta {
        return Err(AppError::BadRequest(
            "insufficient_balance_for_reversal".into(),
        ));
    }

    apply_wallet_delta(tx, w.id, w.version, delta, w.balance).await?;

    let updated = tx_find_wallet_by_id(tx, w.id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found")))?;

    let entry_type = if original.entry_type == WalletEntryType::Credit {
        WalletEntryType::Debit
    } else {
        WalletEntryType::Credit
    };
    insert_tx(
        tx,
        updated.id,
        original.user_id,
        entry_type,
        original.amount,
        updated.balance,
        WalletTxType::Refund,
        &original.currency,
        reversal_tx_no,
        Some(original.id),
        original.reference_type,
        original.reference_id.as_deref(),
        None,
        Some(&serde_json::json!({"reversal": true}).to_string()),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn credit_wallet(
    pool: &crate::db::Pool,
    user_id: i64,
    currency: &str,
    amount: i64,
    tx_type: WalletTxType,
    transaction_no: &str,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    if amount <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive".into()));
    }

    if let Some(existing) =
        crate::models::wallet_transaction::find_tx_by_transaction_no(pool, transaction_no).await?
    {
        return Ok(existing);
    }

    crate::in_transaction!(pool, tx, {
        if let Some(existing) = tx_find_tx_by_transaction_no(&mut tx, transaction_no).await? {
            return Ok(existing);
        }

        ensure_currency_active(&mut tx, currency).await?;

        let w = tx_find_or_create(&mut tx, user_id, currency).await?;
        if w.status != WalletStatus::Active {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        apply_wallet_delta(&mut tx, w.id, w.version, amount, w.balance).await?;

        let updated = tx_find_wallet_by_id(&mut tx, w.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found after update")))?;

        insert_tx(
            &mut tx,
            updated.id,
            user_id,
            WalletEntryType::Credit,
            amount,
            updated.balance,
            tx_type,
            currency,
            transaction_no,
            None,
            reference_type,
            reference_id,
            None,
            metadata,
        )
        .await
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn debit_wallet(
    pool: &crate::db::Pool,
    user_id: i64,
    currency: &str,
    amount: i64,
    tx_type: WalletTxType,
    transaction_no: &str,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    if amount <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive".into()));
    }

    if let Some(existing) =
        crate::models::wallet_transaction::find_tx_by_transaction_no(pool, transaction_no).await?
    {
        return Ok(existing);
    }

    crate::in_transaction!(pool, tx, {
        if let Some(existing) = tx_find_tx_by_transaction_no(&mut tx, transaction_no).await? {
            return Ok(existing);
        }

        ensure_currency_active(&mut tx, currency).await?;

        let w = tx_find_or_create(&mut tx, user_id, currency).await?;
        if w.status != WalletStatus::Active {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        apply_wallet_delta(&mut tx, w.id, w.version, -amount, w.balance).await?;

        let updated = tx_find_wallet_by_id(&mut tx, w.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found after update")))?;

        insert_tx(
            &mut tx,
            updated.id,
            user_id,
            WalletEntryType::Debit,
            amount,
            updated.balance,
            tx_type,
            currency,
            transaction_no,
            None,
            reference_type,
            reference_id,
            None,
            metadata,
        )
        .await
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn transfer(
    pool: &crate::db::Pool,
    from_user_id: i64,
    to_user_id: i64,
    currency: &str,
    amount: i64,
    transaction_no: &str,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<(WalletTransaction, WalletTransaction)> {
    if amount <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive".into()));
    }

    if from_user_id == to_user_id {
        return Err(AppError::BadRequest(
            "cannot_transfer_to_same_wallet".into(),
        ));
    }

    if let Some(existing) =
        crate::models::wallet_transaction::find_tx_by_transaction_no(pool, transaction_no).await?
    {
        let pair_no = format!("{transaction_no}_in");
        let incoming =
            crate::models::wallet_transaction::find_tx_by_transaction_no(pool, &pair_no).await?;
        let incoming = incoming
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("transfer pair incomplete")))?;
        return Ok((existing, incoming));
    }

    crate::in_transaction!(pool, tx, {
        if tx_find_tx_by_transaction_no(&mut tx, transaction_no)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict("duplicate_transaction".into()));
        }

        ensure_currency_active(&mut tx, currency).await?;

        let from_wallet = tx_find_or_create(&mut tx, from_user_id, currency).await?;
        let to_wallet = tx_find_or_create(&mut tx, to_user_id, currency).await?;

        if from_wallet.id == to_wallet.id {
            return Err(AppError::BadRequest(
                "cannot_transfer_to_same_wallet".into(),
            ));
        }

        if from_wallet.status != WalletStatus::Active || to_wallet.status != WalletStatus::Active {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        apply_wallet_delta(
            &mut tx,
            from_wallet.id,
            from_wallet.version,
            -amount,
            from_wallet.balance,
        )
        .await?;
        apply_wallet_delta(
            &mut tx,
            to_wallet.id,
            to_wallet.version,
            amount,
            to_wallet.balance,
        )
        .await?;

        let updated_from = tx_find_wallet_by_id(&mut tx, from_wallet.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found")))?;
        let updated_to = tx_find_wallet_by_id(&mut tx, to_wallet.id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wallet not found")))?;

        let out_tx = insert_tx(
            &mut tx,
            updated_from.id,
            from_user_id,
            WalletEntryType::Debit,
            amount,
            updated_from.balance,
            WalletTxType::TransferOut,
            currency,
            transaction_no,
            None,
            reference_type,
            reference_id,
            Some(updated_to.id),
            metadata,
        )
        .await?;

        let in_no = format!("{transaction_no}_in");
        let in_tx = insert_tx(
            &mut tx,
            updated_to.id,
            to_user_id,
            WalletEntryType::Credit,
            amount,
            updated_to.balance,
            WalletTxType::TransferIn,
            currency,
            &in_no,
            None,
            reference_type,
            reference_id,
            Some(updated_from.id),
            metadata,
        )
        .await?;

        Ok((out_tx, in_tx))
    })
}

pub async fn reverse_transaction(
    pool: &crate::db::Pool,
    original_tx_id: i64,
    transaction_no: &str,
) -> AppResult<WalletTransaction> {
    if let Some(existing) =
        crate::models::wallet_transaction::find_tx_by_transaction_no(pool, transaction_no).await?
    {
        return Ok(existing);
    }

    crate::in_transaction!(pool, tx, {
        if let Some(existing) = tx_find_tx_by_transaction_no(&mut tx, transaction_no).await? {
            return Ok(existing);
        }

        let original = tx_find_tx_by_id(&mut tx, original_tx_id)
            .await?
            .ok_or_else(|| AppError::not_found("transaction"))?;

        if original.tx_type == WalletTxType::Refund {
            return Err(AppError::BadRequest("cannot_reverse_reversal".into()));
        }

        if tx_has_reversal_for(&mut tx, original_tx_id).await? {
            return Err(AppError::BadRequest("already_reversed".into()));
        }

        let refund_tx = reverse_single_tx(&mut tx, &original, transaction_no).await?;

        let original_tx_type = original.tx_type;
        if original_tx_type == WalletTxType::TransferOut
            || original_tx_type == WalletTxType::TransferIn
        {
            let pair_no = if original_tx_type == WalletTxType::TransferOut {
                format!("{}_in", original.transaction_no)
            } else {
                original
                    .transaction_no
                    .strip_suffix("_in")
                    .ok_or_else(|| {
                        AppError::Internal(anyhow::anyhow!("invalid transfer_in transaction_no"))
                    })?
                    .to_string()
            };

            let pair = tx_find_tx_by_transaction_no(&mut tx, &pair_no)
                .await?
                .ok_or_else(|| AppError::Internal(anyhow::anyhow!("transfer pair not found")))?;

            if tx_has_reversal_for(&mut tx, pair.id).await? {
                return Err(AppError::BadRequest(
                    "transfer_pair_already_reversed".into(),
                ));
            }

            let pair_reversal_no = format!("{transaction_no}_pair");
            reverse_single_tx(&mut tx, &pair, &pair_reversal_no).await?;
        }

        Ok(refund_tx)
    })
}

#[allow(clippy::too_many_arguments)]
async fn insert_tx(
    tx: &mut DbConnection,
    wallet_id: i64,
    user_id: i64,
    entry_type: WalletEntryType,
    amount: i64,
    balance_after: i64,
    tx_type: WalletTxType,
    currency: &str,
    transaction_no: &str,
    related_tx_id: Option<i64>,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    counterparty_wallet_id: Option<i64>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    debug_assert!(balance_after >= 0, "balance_after must be non-negative");
    check_schema!(
        "wallet_transactions",
        "document_id",
        "wallet_id",
        "user_id",
        "entry_type",
        "amount",
        "balance_after",
        "tx_type",
        "currency",
        "transaction_no",
        "related_tx_id",
        "reference_type",
        "reference_id",
        "counterparty_wallet_id",
        "metadata",
        "created_at"
    );
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO wallet_transactions (document_id, wallet_id, user_id, entry_type, amount, balance_after, tx_type, currency, transaction_no, related_tx_id, reference_type, reference_id, counterparty_wallet_id, metadata, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        ph(9),
        ph(10),
        ph(11),
        ph(12),
        ph(13),
        ph(14),
        ph(15)
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

    let sql = format!(
        "SELECT * FROM wallet_transactions WHERE document_id = {}",
        ph(1)
    );
    let row = sqlx::query_as::<_, WalletTransaction>(&sql)
        .bind(&document_id)
        .fetch_one(&mut *tx)
        .await?;

    Ok(row)
}

async fn enrich_related_id(
    pool: &crate::db::Pool,
    related_id: Option<i64>,
) -> AppResult<Option<String>> {
    if let Some(rid) = related_id
        && let Some(related) = crate::models::wallet_transaction::find_tx_by_id(pool, rid).await?
    {
        return Ok(Some(related.document_id));
    }
    Ok(None)
}

pub async fn tx_to_response(
    pool: &crate::db::Pool,
    tx: WalletTransaction,
) -> AppResult<crate::dto::WalletTransactionResponse> {
    let related_doc_id = enrich_related_id(pool, tx.related_tx_id).await?;
    let mut resp = crate::dto::WalletTransactionResponse::from_tx(tx)?;
    resp.related_tx_id = related_doc_id;
    Ok(resp)
}

pub async fn tx_list_to_response(
    pool: &crate::db::Pool,
    rows: Vec<WalletTransaction>,
) -> AppResult<Vec<crate::dto::WalletTransactionResponse>> {
    let related_ids: Vec<i64> = rows.iter().filter_map(|r| r.related_tx_id).collect();

    let doc_id_map =
        crate::models::wallet_transaction::find_document_ids_by_ids(pool, &related_ids).await?;

    let mut responses = Vec::with_capacity(rows.len());
    for row in rows {
        let related_doc_id = row
            .related_tx_id
            .and_then(|rid| doc_id_map.get(&rid).cloned());
        let mut resp = crate::dto::WalletTransactionResponse::from_tx(row)?;
        resp.related_tx_id = related_doc_id;
        responses.push(resp);
    }
    Ok(responses)
}

#[async_trait]
impl WalletService for WalletServiceImpl {
    async fn credit(
        &self,
        user_id: i64,
        currency: &str,
        amount: i64,
        tx_type: WalletTxType,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<WalletTransaction> {
        let tx = credit_wallet(
            &self.pool,
            user_id,
            currency,
            amount,
            tx_type,
            transaction_no,
            reference_type,
            reference_id,
            metadata,
        )
        .await?;
        self.after_credited(&tx);
        Ok(tx)
    }

    async fn debit(
        &self,
        user_id: i64,
        currency: &str,
        amount: i64,
        tx_type: WalletTxType,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<WalletTransaction> {
        let tx = debit_wallet(
            &self.pool,
            user_id,
            currency,
            amount,
            tx_type,
            transaction_no,
            reference_type,
            reference_id,
            metadata,
        )
        .await?;
        self.after_debited(&tx);
        Ok(tx)
    }

    async fn transfer(
        &self,
        from_user_id: i64,
        to_user_id: i64,
        currency: &str,
        amount: i64,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<(WalletTransaction, WalletTransaction)> {
        let (out_tx, in_tx) = transfer(
            &self.pool,
            from_user_id,
            to_user_id,
            currency,
            amount,
            transaction_no,
            reference_type,
            reference_id,
            metadata,
        )
        .await?;
        self.after_debited(&out_tx);
        self.after_credited(&in_tx);
        Ok((out_tx, in_tx))
    }

    async fn reverse_transaction(
        &self,
        original_tx_id: i64,
        transaction_no: &str,
    ) -> AppResult<WalletTransaction> {
        let tx = reverse_transaction(&self.pool, original_tx_id, transaction_no).await?;
        match tx.entry_type {
            crate::models::wallet_transaction::WalletEntryType::Credit => {
                self.after_credited(&tx);
            }
            crate::models::wallet_transaction::WalletEntryType::Debit => {
                self.after_debited(&tx);
            }
        }
        Ok(tx)
    }

    async fn tx_to_response(
        &self,
        tx: WalletTransaction,
    ) -> AppResult<crate::dto::WalletTransactionResponse> {
        tx_to_response(&self.pool, tx).await
    }

    async fn tx_list_to_response(
        &self,
        rows: Vec<WalletTransaction>,
    ) -> AppResult<Vec<crate::dto::WalletTransactionResponse>> {
        tx_list_to_response(&self.pool, rows).await
    }

    async fn find_user_int_id(&self, user_doc_id: &str, tenant_id: Option<&str>) -> AppResult<i64> {
        let user = crate::models::user::find_by_id(&self.pool, user_doc_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;
        Ok(user.id)
    }

    async fn list_wallets_by_user(
        &self,
        user_doc_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<crate::models::wallet::Wallet>> {
        let user_id = self.find_user_int_id(user_doc_id, tenant_id).await?;
        crate::models::wallet::find_by_user(&self.pool, user_id).await
    }

    async fn get_wallet_by_currency(
        &self,
        user_doc_id: &str,
        currency: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<crate::models::wallet::Wallet> {
        let user_id = self.find_user_int_id(user_doc_id, tenant_id).await?;
        crate::models::wallet::find_by_user_and_currency(&self.pool, user_id, currency)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))
    }

    async fn list_transactions_by_wallet(
        &self,
        user_doc_id: &str,
        currency: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
        let user_id = self.find_user_int_id(user_doc_id, tenant_id).await?;
        let w = crate::models::wallet::find_by_user_and_currency(&self.pool, user_id, currency)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))?;
        crate::models::wallet_transaction::find_transactions_by_wallet(
            &self.pool, w.id, page, page_size,
        )
        .await
    }

    async fn list_transactions_by_user(
        &self,
        user_doc_id: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
        let user_id = self.find_user_int_id(user_doc_id, tenant_id).await?;
        crate::models::wallet_transaction::find_transactions_by_user(
            &self.pool, user_id, page, page_size,
        )
        .await
    }

    async fn list_all_wallets(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<crate::models::wallet::Wallet>, i64)> {
        crate::models::wallet::find_all_wallets(&self.pool, page, page_size, tenant_id).await
    }

    async fn list_all_transactions(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
        crate::models::wallet_transaction::find_all_transactions(
            &self.pool, page, page_size, tenant_id,
        )
        .await
    }

    async fn find_tx_by_document_id(
        &self,
        tx_doc_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<WalletTransaction> {
        let tx = crate::models::wallet_transaction::find_tx_by_document_id(&self.pool, tx_doc_id)
            .await?
            .ok_or_else(|| AppError::not_found("transaction"))?;
        if let Some(tid) = tenant_id {
            let wallet = crate::models::wallet::find_by_id(&self.pool, tx.wallet_id)
                .await?
                .ok_or_else(|| AppError::not_found("wallet"))?;
            let user =
                crate::models::user::find_by_id(&self.pool, &wallet.user_id.to_string(), Some(tid))
                    .await?;
            if user.is_none() {
                return Err(AppError::not_found("transaction"));
            }
        }
        Ok(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::app_error::AppError;

    struct TestContext {
        pool: crate::db::Pool,
        _guard: TempDbGuard,
    }

    impl std::ops::Deref for TestContext {
        type Target = crate::db::Pool;
        fn deref(&self) -> &Self::Target {
            &self.pool
        }
    }

    struct TempDbGuard {
        path: std::path::PathBuf,
    }

    impl Drop for TempDbGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let wal = self.path.with_extension("db-wal");
            let _ = std::fs::remove_file(&wal);
            let shm = self.path.with_extension("db-shm");
            let _ = std::fs::remove_file(&shm);
        }
    }

    async fn setup() -> TestContext {
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
        crate::models::currencies::create(&pool, "CNY", "Chinese Yuan", 2)
            .await
            .unwrap();
        crate::models::currencies::create(&pool, "USD", "US Dollar", 2)
            .await
            .unwrap();
        TestContext {
            pool,
            _guard: TempDbGuard { path },
        }
    }

    #[allow(dead_code)]
    async fn seed_currencies(pool: &crate::db::Pool) {
        crate::models::currencies::create(pool, "CNY", "Chinese Yuan", 2)
            .await
            .unwrap();
        crate::models::currencies::create(pool, "USD", "US Dollar", 2)
            .await
            .unwrap();
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

    fn new_tx_no() -> String {
        format!("TX_{}", crate::utils::id::new_document_id())
    }

    // ── credit_wallet ──

    #[tokio::test]
    async fn credit_normal() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;
        let tx_no = new_tx_no();

        let tx = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &tx_no,
            Some(R::Admin),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(tx.entry_type, E::Credit);
        assert_eq!(tx.amount, 500);
        assert_eq!(tx.balance_after, 500);
        assert_eq!(tx.tx_type, T::Recharge);

        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 500);
    }

    #[tokio::test]
    async fn credit_auto_creates_wallet() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        assert!(
            wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
                .await
                .unwrap()
                .is_none()
        );

        let tx_no = new_tx_no();
        let tx1 = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx2 = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(tx1.transaction_no, tx2.transaction_no);
        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 500);
    }

    #[tokio::test]
    async fn credit_amount_zero_rejected() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let err = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            0,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
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
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let err = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            -100,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
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
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            300,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            700,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 1000);
    }

    // ── debit_wallet ──

    #[tokio::test]
    async fn debit_normal() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            400,
            T::Payment,
            &new_tx_no(),
            Some(R::Order),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(tx.entry_type, E::Debit);
        assert_eq!(tx.amount, 400);
        assert_eq!(tx.balance_after, 600);

        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 600);
    }

    #[tokio::test]
    async fn debit_insufficient_balance() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            100,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            200,
            T::Payment,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => {
                assert_eq!(msg, "insufficient_balance_or_concurrent_update")
            }
            _ => panic!("expected BadRequest"),
        }

        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 100);
    }

    #[tokio::test]
    async fn debit_exact_balance() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Payment,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(tx.balance_after, 0);
    }

    #[tokio::test]
    async fn debit_idempotent() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;
        let tx_no = new_tx_no();

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx1 = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            300,
            T::Payment,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let tx2 = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            300,
            T::Payment,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(tx1.transaction_no, tx2.transaction_no);
        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 700);
    }

    #[tokio::test]
    async fn debit_amount_must_be_positive() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let err = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            0,
            T::Payment,
            &new_tx_no(),
            None,
            None,
            None,
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
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let err = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            100,
            T::Payment,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => {
                assert_eq!(msg, "insufficient_balance_or_concurrent_update")
            }
            _ => panic!("expected BadRequest"),
        }
    }

    // ── transfer ──

    #[tokio::test]
    async fn transfer_normal() {
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            from_user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (out_tx, in_tx) = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            300,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(out_tx.entry_type, E::Debit);
        assert_eq!(out_tx.tx_type, T::TransferOut);
        assert_eq!(out_tx.amount, 300);
        assert_eq!(out_tx.balance_after, 700);

        assert_eq!(in_tx.entry_type, E::Credit);
        assert_eq!(in_tx.tx_type, T::TransferIn);
        assert_eq!(in_tx.amount, 300);
        assert_eq!(in_tx.balance_after, 300);

        let from_w = wallet::find_by_user_and_currency(&ctx, from_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_w.balance, 700);
        let to_w = wallet::find_by_user_and_currency(&ctx, to_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(to_w.balance, 300);
    }

    #[tokio::test]
    async fn transfer_insufficient_balance() {
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            from_user.id,
            "CNY",
            100,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            200,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap_err();

        match err {
            AppError::BadRequest(msg) => {
                assert_eq!(msg, "insufficient_balance_or_concurrent_update")
            }
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn transfer_to_self_rejected() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = transfer(
            &ctx,
            user.id,
            user.id,
            "CNY",
            100,
            &new_tx_no(),
            None,
            None,
            None,
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
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            from_user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (out1, in1) = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            300,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let (out2, in2) = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            300,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(out1.transaction_no, out2.transaction_no);
        assert_eq!(in1.transaction_no, in2.transaction_no);

        let from_w = wallet::find_by_user_and_currency(&ctx, from_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_w.balance, 700);
    }

    #[tokio::test]
    async fn transfer_amount_must_be_positive() {
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        let err = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            0,
            &new_tx_no(),
            None,
            None,
            None,
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
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let original = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let rev_tx = reverse_transaction(&ctx, original.id, &new_tx_no())
            .await
            .unwrap();

        assert_eq!(rev_tx.entry_type, E::Debit);
        assert_eq!(rev_tx.amount, 500);
        assert_eq!(rev_tx.balance_after, 0);
        assert_eq!(rev_tx.related_tx_id, Some(original.id));

        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 0);
    }

    #[tokio::test]
    async fn reverse_debit() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let original = debit_wallet(
            &ctx,
            user.id,
            "CNY",
            300,
            T::Payment,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let rev_tx = reverse_transaction(&ctx, original.id, &new_tx_no())
            .await
            .unwrap();

        assert_eq!(rev_tx.entry_type, E::Credit);
        assert_eq!(rev_tx.amount, 300);
        assert_eq!(rev_tx.balance_after, 1000);

        let w = wallet::find_by_user_and_currency(&ctx, user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w.balance, 1000);
    }

    #[tokio::test]
    async fn reverse_idempotent() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let original = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let rev_no = new_tx_no();
        let rev1 = reverse_transaction(&ctx, original.id, &rev_no)
            .await
            .unwrap();
        let rev2 = reverse_transaction(&ctx, original.id, &rev_no)
            .await
            .unwrap();
        assert_eq!(rev1.transaction_no, rev2.transaction_no);
    }

    #[tokio::test]
    async fn reverse_cannot_reverse_reversal() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let original = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let rev = reverse_transaction(&ctx, original.id, &new_tx_no())
            .await
            .unwrap();

        let err = reverse_transaction(&ctx, rev.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "cannot_reverse_reversal"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn reverse_already_reversed_rejected() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        let original = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        reverse_transaction(&ctx, original.id, &new_tx_no())
            .await
            .unwrap();

        let err = reverse_transaction(&ctx, original.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "already_reversed"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn reverse_insufficient_balance_for_debit_reversal() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let credit_tx = credit_wallet(
            &ctx,
            user.id,
            "CNY",
            500,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        debit_wallet(
            &ctx,
            user.id,
            "CNY",
            1400,
            T::Payment,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = reverse_transaction(&ctx, credit_tx.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "insufficient_balance_for_reversal"),
            _ => panic!("expected BadRequest"),
        }
    }

    #[tokio::test]
    async fn reverse_nonexistent_transaction() {
        let ctx = setup().await;

        let err = reverse_transaction(&ctx, 99999, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::NotFound(_) => {}
            _ => panic!("expected NotFound"),
        }
    }

    // ── transfer reversal (Bug 2 fix) ──

    #[tokio::test]
    async fn reverse_transfer_reverses_both_legs() {
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            from_user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (out_tx, _in_tx) = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            300,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let rev_no = new_tx_no();
        let rev = reverse_transaction(&ctx, out_tx.id, &rev_no).await.unwrap();

        assert_eq!(rev.entry_type, E::Credit);
        assert_eq!(rev.amount, 300);
        assert_eq!(rev.tx_type, T::Refund);
        assert_eq!(rev.related_tx_id, Some(out_tx.id));

        let from_w = wallet::find_by_user_and_currency(&ctx, from_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_w.balance, 1000);

        let to_w = wallet::find_by_user_and_currency(&ctx, to_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(to_w.balance, 0);
    }

    #[tokio::test]
    async fn reverse_transfer_in_also_reverses_both_legs() {
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            from_user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (_out_tx, in_tx) = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            300,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let rev = reverse_transaction(&ctx, in_tx.id, &new_tx_no())
            .await
            .unwrap();
        assert_eq!(rev.tx_type, T::Refund);

        let from_w = wallet::find_by_user_and_currency(&ctx, from_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_w.balance, 1000);

        let to_w = wallet::find_by_user_and_currency(&ctx, to_user.id, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(to_w.balance, 0);
    }

    #[tokio::test]
    async fn reverse_transfer_insufficient_receiver_balance() {
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            from_user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (out_tx, _in_tx) = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            500,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // receiver spends the money
        debit_wallet(
            &ctx,
            to_user.id,
            "CNY",
            500,
            T::Payment,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = reverse_transaction(&ctx, out_tx.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "insufficient_balance_for_reversal"),
            _ => panic!("expected BadRequest, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn reverse_transfer_pair_already_reversed() {
        let ctx = setup().await;
        let from_user = insert_user(&ctx).await;
        let to_user = insert_user(&ctx).await;

        credit_wallet(
            &ctx,
            from_user.id,
            "CNY",
            1000,
            T::Recharge,
            &new_tx_no(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let tx_no = new_tx_no();
        let (out_tx, in_tx) = transfer(
            &ctx,
            from_user.id,
            to_user.id,
            "CNY",
            300,
            &tx_no,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // reverse the out_tx first
        reverse_transaction(&ctx, out_tx.id, &new_tx_no())
            .await
            .unwrap();

        // trying to reverse in_tx should fail because it was already reversed as part of the pair
        let err = reverse_transaction(&ctx, in_tx.id, &new_tx_no())
            .await
            .unwrap_err();

        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "already_reversed"),
            _ => panic!("expected BadRequest"),
        }
    }
}

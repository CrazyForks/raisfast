use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::db::pool::DbConnection;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::currencies;
use crate::models::wallet;
use crate::models::wallet::WalletStatus;
use crate::models::wallet_transaction;
use crate::models::wallet_transaction::WalletTransaction;
use crate::models::wallet_transaction::{WalletEntryType, WalletReferenceType, WalletTxType};
use crate::types::snowflake_id::SnowflakeId;

#[async_trait]
pub trait WalletService: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn credit(
        &self,
        auth: &AuthUser,
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
        auth: &AuthUser,
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
        auth: &AuthUser,
        from_user_id: SnowflakeId,
        to_user_id: SnowflakeId,
        currency: &str,
        amount: i64,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<(WalletTransaction, WalletTransaction)>;

    async fn reverse_transaction(
        &self,
        original_tx_id: SnowflakeId,
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

    async fn list_wallets_by_user(
        &self,
        user_id: SnowflakeId,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<crate::models::wallet::Wallet>>;

    async fn get_wallet_by_currency(
        &self,
        user_id: SnowflakeId,
        currency: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<crate::models::wallet::Wallet>;

    async fn list_transactions_by_wallet(
        &self,
        user_id: SnowflakeId,
        currency: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)>;

    async fn list_transactions_by_user(
        &self,
        user_id: SnowflakeId,
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

    async fn find_tx_by_id(
        &self,
        tx_id: &str,
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

async fn ensure_currency_active(
    tx: &mut DbConnection,
    currency: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    currencies::find_by_code_tx(tx, currency, tenant_id)
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

async fn tx_find_wallet_by_id(
    tx: &mut DbConnection,
    id: SnowflakeId,
) -> AppResult<Option<wallet::Wallet>> {
    wallet::tx_find_by_id(tx, id).await
}

async fn tx_find_or_create(
    tx: &mut DbConnection,
    user_id: SnowflakeId,
    currency: &str,
) -> AppResult<wallet::Wallet> {
    wallet::tx_find_or_create(tx, user_id, currency).await
}

async fn tx_find_tx_by_id(
    tx: &mut DbConnection,
    id: SnowflakeId,
) -> AppResult<Option<WalletTransaction>> {
    wallet_transaction::tx_find_by_id(tx, id).await
}

async fn tx_find_tx_by_transaction_no(
    tx: &mut DbConnection,
    transaction_no: &str,
) -> AppResult<Option<WalletTransaction>> {
    wallet_transaction::tx_find_by_transaction_no(tx, transaction_no).await
}

async fn tx_has_reversal_for(tx: &mut DbConnection, related_tx_id: SnowflakeId) -> AppResult<bool> {
    wallet_transaction::tx_has_reversal_for(tx, related_tx_id).await
}

async fn apply_wallet_delta(
    tx: &mut DbConnection,
    wallet_id: SnowflakeId,
    version: i64,
    delta: i64,
    current_balance: i64,
) -> AppResult<()> {
    wallet::apply_wallet_delta(tx, wallet_id, version, delta, current_balance).await
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
        .ok_or_else(|| AppError::not_found("wallet"))?;

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
        original.reference_id.clone(),
        None,
        Some(serde_json::json!({"reversal": true}).to_string()),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn credit_wallet(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    currency: &str,
    amount: i64,
    tx_type: WalletTxType,
    transaction_no: &str,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    let user_id = auth.ensure_snowflake_user_id()?;
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

        ensure_currency_active(&mut tx, currency, auth.tenant_id()).await?;

        let w = tx_find_or_create(&mut tx, user_id, currency).await?;
        if w.status != WalletStatus::Active {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        apply_wallet_delta(&mut tx, w.id, w.version, amount, w.balance).await?;

        let updated = tx_find_wallet_by_id(&mut tx, w.id)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))?;

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
            reference_id.map(|s| s.to_string()),
            None,
            metadata.map(|s| s.to_string()),
        )
        .await
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn debit_wallet(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    currency: &str,
    amount: i64,
    tx_type: WalletTxType,
    transaction_no: &str,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    let user_id = auth.ensure_snowflake_user_id()?;
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

        ensure_currency_active(&mut tx, currency, auth.tenant_id()).await?;

        let w = tx_find_or_create(&mut tx, user_id, currency).await?;
        if w.status != WalletStatus::Active {
            return Err(AppError::BadRequest("wallet_frozen".into()));
        }

        apply_wallet_delta(&mut tx, w.id, w.version, -amount, w.balance).await?;

        let updated = tx_find_wallet_by_id(&mut tx, w.id)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))?;

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
            reference_id.map(|s| s.to_string()),
            None,
            metadata.map(|s| s.to_string()),
        )
        .await
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn transfer(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    from_user_id: SnowflakeId,
    to_user_id: SnowflakeId,
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
        let incoming = incoming.ok_or_else(|| AppError::not_found("transaction"))?;
        return Ok((existing, incoming));
    }

    crate::in_transaction!(pool, tx, {
        if tx_find_tx_by_transaction_no(&mut tx, transaction_no)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict("duplicate_transaction".into()));
        }

        ensure_currency_active(&mut tx, currency, auth.tenant_id()).await?;

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
            .ok_or_else(|| AppError::not_found("wallet"))?;
        let updated_to = tx_find_wallet_by_id(&mut tx, to_wallet.id)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))?;

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
            reference_id.map(|s| s.to_string()),
            Some(updated_to.id),
            metadata.map(|s| s.to_string()),
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
            reference_id.map(|s| s.to_string()),
            Some(updated_from.id),
            metadata.map(|s| s.to_string()),
        )
        .await?;

        Ok((out_tx, in_tx))
    })
}

pub async fn reverse_transaction(
    pool: &crate::db::Pool,
    original_tx_id: SnowflakeId,
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
                        AppError::BadRequest("invalid_transfer_in_transaction_no".into())
                    })?
                    .to_string()
            };

            let pair = tx_find_tx_by_transaction_no(&mut tx, &pair_no)
                .await?
                .ok_or_else(|| AppError::not_found("transaction"))?;

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
    debug_assert!(balance_after >= 0, "balance_after must be non-negative");
    let row = wallet_transaction::tx_insert(
        &mut *tx,
        wallet_id,
        user_id,
        entry_type,
        amount,
        balance_after,
        tx_type,
        currency,
        transaction_no,
        related_tx_id,
        reference_type,
        reference_id,
        counterparty_wallet_id,
        metadata,
    )
    .await?;
    Ok(row)
}

async fn enrich_related_id(
    pool: &crate::db::Pool,
    related_id: Option<SnowflakeId>,
) -> AppResult<Option<String>> {
    if let Some(rid) = related_id
        && let Some(related) = crate::models::wallet_transaction::find_tx_by_id(pool, rid).await?
    {
        return Ok(Some(related.id.to_string()));
    }
    Ok(None)
}

pub async fn tx_to_response(
    pool: &crate::db::Pool,
    tx: WalletTransaction,
) -> AppResult<crate::dto::WalletTransactionResponse> {
    let related_id = enrich_related_id(pool, tx.related_tx_id).await?;
    let mut resp = crate::dto::WalletTransactionResponse::from_tx(tx)?;
    resp.related_tx_id = related_id;
    Ok(resp)
}

pub async fn tx_list_to_response(
    _pool: &crate::db::Pool,
    rows: Vec<WalletTransaction>,
) -> AppResult<Vec<crate::dto::WalletTransactionResponse>> {
    let related_ids: Vec<SnowflakeId> = rows.iter().filter_map(|r| r.related_tx_id).collect();

    let id_map: std::collections::HashMap<SnowflakeId, String> = related_ids
        .into_iter()
        .map(|id| (id, id.to_string()))
        .collect();

    let mut responses = Vec::with_capacity(rows.len());
    for row in rows {
        let related_id = row.related_tx_id.and_then(|rid| id_map.get(&rid).cloned());
        let mut resp = crate::dto::WalletTransactionResponse::from_tx(row)?;
        resp.related_tx_id = related_id;
        responses.push(resp);
    }
    Ok(responses)
}

#[async_trait]
impl WalletService for WalletServiceImpl {
    async fn credit(
        &self,
        auth: &AuthUser,
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
            auth,
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
        auth: &AuthUser,
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
            auth,
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
        auth: &AuthUser,
        from_user_id: SnowflakeId,
        to_user_id: SnowflakeId,
        currency: &str,
        amount: i64,
        transaction_no: &str,
        reference_type: Option<WalletReferenceType>,
        reference_id: Option<&str>,
        metadata: Option<&str>,
    ) -> AppResult<(WalletTransaction, WalletTransaction)> {
        let (out_tx, in_tx) = transfer(
            &self.pool,
            auth,
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
        original_tx_id: SnowflakeId,
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

    async fn list_wallets_by_user(
        &self,
        user_id: SnowflakeId,
        _tenant_id: Option<&str>,
    ) -> AppResult<Vec<crate::models::wallet::Wallet>> {
        crate::models::wallet::find_by_user(&self.pool, user_id).await
    }

    async fn get_wallet_by_currency(
        &self,
        user_id: SnowflakeId,
        currency: &str,
        _tenant_id: Option<&str>,
    ) -> AppResult<crate::models::wallet::Wallet> {
        crate::models::wallet::find_by_user_and_currency(&self.pool, user_id, currency)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))
    }

    async fn list_transactions_by_wallet(
        &self,
        user_id: SnowflakeId,
        currency: &str,
        page: i64,
        page_size: i64,
        _tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
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
        user_id: SnowflakeId,
        page: i64,
        page_size: i64,
        _tenant_id: Option<&str>,
    ) -> AppResult<(Vec<WalletTransaction>, i64)> {
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

    async fn find_tx_by_id(
        &self,
        tx_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<WalletTransaction> {
        let id = crate::types::snowflake_id::parse_id(tx_id)?;
        let tx = crate::models::wallet_transaction::find_tx_by_id(&self.pool, id)
            .await?
            .ok_or_else(|| AppError::not_found("transaction"))?;
        if let Some(tid) = tenant_id {
            let wallet = crate::models::wallet::find_by_id(&self.pool, tx.wallet_id)
                .await?
                .ok_or_else(|| AppError::not_found("wallet"))?;
            let user =
                crate::models::user::find_by_id(&self.pool, wallet.user_id, Some(tid)).await?;
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
    use crate::middleware::auth::AuthUser;
    use crate::models::user::UserRole;

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
        #[cfg(feature = "db-sqlite")]
        {
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        TestContext {
            pool,
            _guard: TempDbGuard { path },
        }
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

    fn new_tx_no() -> String {
        format!("TX_{}", crate::utils::id::new_id())
    }

    // ── credit_wallet ──

    #[tokio::test]
    async fn credit_normal() {
        let ctx = setup().await;
        let user = insert_user(&ctx).await;
        let tx_no = new_tx_no();

        let tx = credit_wallet(
            &ctx,
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(user.id.0, UserRole::Reader, "default"),
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

        let err = reverse_transaction(&ctx, SnowflakeId(99999), &new_tx_no())
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(to_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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
            &AuthUser::new_test(from_user.id.0, UserRole::Reader, "default"),
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

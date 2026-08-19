use crate::types::price::Price;
use crate::types::snowflake_id::SnowflakeId;
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::errors::app_error::AppResult;
use crate::models::wallet::{Wallet, WalletStatus};
use crate::models::wallet_transaction::{
    WalletEntryType, WalletReferenceType, WalletTransaction, WalletTxType,
};
use crate::utils::tz::Timestamp;

/// Wallet response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletResponse {
    pub id: SnowflakeId,
    pub currency: String,
    pub balance: Price,
    pub status: WalletStatus,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl WalletResponse {
    pub fn from_wallet(w: Wallet) -> AppResult<Self> {
        Ok(Self {
            status: w.status,
            id: w.id,
            currency: w.currency,
            balance: w.balance,
            created_at: w.created_at,
            updated_at: w.updated_at,
        })
    }
}

/// Transaction record response
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletTransactionResponse {
    pub id: SnowflakeId,
    pub entry_type: WalletEntryType,
    pub amount: Price,
    pub balance_after: Price,
    pub tx_type: WalletTxType,
    pub currency: String,
    pub transaction_no: String,
    pub reference_type: Option<WalletReferenceType>,
    pub reference_id: Option<String>,
    pub metadata: Option<String>,
    pub related_tx_id: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
}

impl WalletTransactionResponse {
    pub fn from_tx(tx: WalletTransaction) -> AppResult<Self> {
        Ok(Self {
            entry_type: tx.entry_type,
            tx_type: tx.tx_type,
            reference_type: tx.reference_type,
            id: tx.id,
            amount: tx.amount,
            balance_after: tx.balance_after,
            currency: tx.currency,
            transaction_no: tx.transaction_no,
            reference_id: tx.reference_id,
            metadata: tx.metadata,
            related_tx_id: None,
            created_at: tx.created_at,
        })
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct AdminWalletOperationRequest {
    pub user_id: SnowflakeId,
    #[validate(
        length(min = 1),
        custom(function = "crate::dto::validate_currency_code")
    )]
    pub currency: String,
    #[validate(length(min = 1))]
    pub transaction_no: String,
    pub amount: Price,
    pub reference_type: Option<crate::models::wallet_transaction::WalletReferenceType>,
    pub reference_id: Option<String>,
    #[schema(value_type = Option<String>)]
    pub metadata: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ReversalRequest {
    #[validate(length(min = 1))]
    pub transaction_no: String,
}

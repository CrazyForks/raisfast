use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::errors::app_error::{AppError, AppResult};
use crate::models::wallet::{Wallet, WalletStatus};
use crate::models::wallet_transaction::{
    WalletEntryType, WalletReferenceType, WalletTransaction, WalletTxType,
};
use crate::utils::tz::Timestamp;

fn map_enum_err(e: String) -> AppError {
    AppError::Internal(anyhow::anyhow!(e))
}

/// 钱包响应
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletResponse {
    pub id: String,
    pub currency: String,
    pub balance: i64,
    pub status: WalletStatus,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl WalletResponse {
    pub fn from_wallet(w: Wallet) -> AppResult<Self> {
        Ok(Self {
            status: w.status_enum().map_err(map_enum_err)?,
            id: w.document_id,
            currency: w.currency,
            balance: w.balance,
            created_at: w.created_at,
            updated_at: w.updated_at,
        })
    }
}

/// 交易流水响应
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletTransactionResponse {
    pub id: String,
    pub entry_type: WalletEntryType,
    pub amount: i64,
    pub balance_after: i64,
    pub tx_type: WalletTxType,
    pub currency: String,
    pub transaction_no: String,
    pub related_tx_id: Option<String>,
    pub reference_type: Option<WalletReferenceType>,
    pub reference_id: Option<String>,
    pub metadata: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
}

impl WalletTransactionResponse {
    pub fn from_tx(tx: WalletTransaction) -> AppResult<Self> {
        Ok(Self {
            entry_type: tx.entry_type_enum().map_err(map_enum_err)?,
            tx_type: tx.tx_type_enum().map_err(map_enum_err)?,
            reference_type: tx.reference_type_enum().map_err(map_enum_err)?,
            id: tx.document_id,
            amount: tx.amount,
            balance_after: tx.balance_after,
            currency: tx.currency,
            transaction_no: tx.transaction_no,
            related_tx_id: None,
            reference_id: tx.reference_id,
            metadata: tx.metadata,
            created_at: tx.created_at,
        })
    }
}

/// 管理员加款/扣款请求
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct AdminWalletOperationRequest {
    pub user_id: String,
    #[validate(
        length(min = 1),
        custom(function = "crate::dto::validate_currency_code")
    )]
    pub currency: String,
    #[validate(length(min = 1))]
    pub transaction_no: String,
    #[validate(range(min = 1))]
    pub amount: i64,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    #[schema(value_type = Option<String>)]
    pub metadata: Option<String>,
}

/// 冲正请求
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ReversalRequest {
    #[validate(length(min = 1))]
    pub transaction_no: String,
}

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

use crate::models::wallet::Wallet;
use crate::models::wallet_transaction::WalletTransaction;
use crate::utils::tz::Timestamp;

/// 钱包响应
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletResponse {
    pub id: String,
    pub currency: String,
    pub balance: i64,
    pub status: String,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl From<Wallet> for WalletResponse {
    fn from(w: Wallet) -> Self {
        Self {
            id: w.document_id,
            currency: w.currency,
            balance: w.balance,
            status: w.status,
            created_at: w.created_at,
            updated_at: w.updated_at,
        }
    }
}

/// 交易流水响应
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct WalletTransactionResponse {
    pub id: String,
    pub entry_type: String,
    pub amount: i64,
    pub balance_after: i64,
    pub tx_type: String,
    pub currency: String,
    pub transaction_no: String,
    pub related_tx_id: Option<i64>,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub counterparty_wallet_id: Option<i64>,
    #[schema(value_type = Option<String>)]
    pub metadata: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
}

impl From<WalletTransaction> for WalletTransactionResponse {
    fn from(t: WalletTransaction) -> Self {
        Self {
            id: t.document_id,
            entry_type: t.entry_type,
            amount: t.amount,
            balance_after: t.balance_after,
            tx_type: t.tx_type,
            currency: t.currency,
            transaction_no: t.transaction_no,
            related_tx_id: t.related_tx_id,
            reference_type: t.reference_type,
            reference_id: t.reference_id,
            counterparty_wallet_id: t.counterparty_wallet_id,
            metadata: t.metadata,
            created_at: t.created_at,
        }
    }
}

/// 管理员加款/扣款请求
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct AdminWalletOperationRequest {
    pub user_id: String,
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

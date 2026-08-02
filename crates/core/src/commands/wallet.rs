use crate::models::wallet_transaction::{WalletReferenceType, WalletTxType};
use crate::types::snowflake_id::SnowflakeId;

pub struct CreateWalletOutboxCmd {
    pub user_id: SnowflakeId,
    pub currency: String,
    pub amount: i64,
    pub entry_type: String,
    pub tx_type: WalletTxType,
    pub transaction_no: String,
    pub reference_type: Option<WalletReferenceType>,
    pub reference_id: Option<String>,
    pub metadata: Option<String>,
}

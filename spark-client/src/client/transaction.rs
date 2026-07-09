use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxCursor(pub(super) String);

impl From<String> for TxCursor {
    fn from(cursor: String) -> Self {
        Self(cursor)
    }
}

impl From<TxCursor> for String {
    fn from(cursor: TxCursor) -> Self {
        cursor.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparkTransactionType {
    Mint,
    Burn,
}

#[derive(Debug, Clone)]
pub struct SparkTransaction {
    pub id: String,
    pub cursor: TxCursor,
    pub created_at: DateTime<Utc>,
    pub amount: u128,
    pub status: breez_sdk_spark::PaymentStatus,
    pub payment_type: breez_sdk_spark::PaymentType,
    pub tx_type: SparkTransactionType,
    pub token_identifier: String,
}

#[derive(Debug, Clone)]
pub struct SparkTransactions {
    pub cursor: Option<TxCursor>,
    pub list: Vec<SparkTransaction>,
    pub has_more: bool,
}

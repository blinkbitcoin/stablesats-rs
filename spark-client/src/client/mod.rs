mod config;
mod transaction;

use std::sync::Arc;

use breez_sdk_spark::{
    default_server_config, postgres_storage, AssetFilter, ListPaymentsRequest, Payment,
    PaymentDetails, PaymentDetailsFilter, PaymentStatus, PostgresStorageConfig, SdkBuilder, Seed,
    TokenTransactionType,
};
use chrono::{DateTime, Utc};
use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::error::SparkClientError;

pub use config::*;
pub use transaction::*;

#[derive(Clone)]
pub struct SparkClient {
    sdk: Arc<breez_sdk_spark::BreezSdk>,
    config: SparkClientConfig,
}

impl SparkClient {
    pub async fn connect(config: SparkClientConfig) -> Result<Self, SparkClientError> {
        validate_config(&config)?;

        let mut sdk_config = default_server_config(config.network.clone().into());
        sdk_config.api_key = Some(config.api_key.clone());

        let seed = Seed::Mnemonic {
            mnemonic: config.mnemonic.clone(),
            passphrase: config.mnemonic_passphrase.clone(),
        };

        let storage_config =
            PostgresStorageConfig::with_defaults(config.postgres_connection_string.clone());
        let storage_backend = postgres_storage(storage_config)?;

        let sdk = SdkBuilder::new(sdk_config, seed)
            .with_storage_backend(storage_backend)
            .build()
            .await?;

        Ok(Self {
            sdk: Arc::new(sdk),
            config,
        })
    }

    #[instrument(name = "spark_client.transactions_list", skip(self), err)]
    pub async fn transactions_list(
        &self,
        cursor: Option<TxCursor>,
    ) -> Result<SparkTransactions, SparkClientError> {
        let start_offset = start_offset_from_cursor(cursor)?;
        let limit = self.config.page_size();

        let response = self
            .sdk
            .list_payments(ListPaymentsRequest {
                status_filter: Some(vec![PaymentStatus::Completed]),
                asset_filter: Some(AssetFilter::Token {
                    token_identifier: Some(self.config.token_identifier.clone()),
                }),
                payment_details_filter: Some(vec![
                    PaymentDetailsFilter::Token {
                        conversion_refund_needed: None,
                        tx_hash: None,
                        tx_type: Some(TokenTransactionType::Mint),
                    },
                    PaymentDetailsFilter::Token {
                        conversion_refund_needed: None,
                        tx_hash: None,
                        tx_type: Some(TokenTransactionType::Burn),
                    },
                ]),
                offset: Some(start_offset),
                limit: Some(limit),
                sort_ascending: Some(true),
                ..Default::default()
            })
            .await?;

        let fetched_len = response.payments.len();
        let mut list = Vec::new();

        for (index, payment) in response.payments.into_iter().enumerate() {
            let absolute_offset = start_offset.saturating_add(index as u32);
            if let Some(tx) = map_payment(payment, absolute_offset, &self.config.token_identifier)?
            {
                list.push(tx);
            }
        }

        let cursor = list.last().map(|tx| tx.cursor.clone());

        Ok(SparkTransactions {
            cursor,
            list,
            has_more: has_more_for_page_len(fetched_len, limit),
        })
    }
}

fn validate_config(config: &SparkClientConfig) -> Result<(), SparkClientError> {
    if config.api_key.trim().is_empty() {
        return Err(SparkClientError::Authentication(
            "Empty API key".to_string(),
        ));
    }
    if config.mnemonic.trim().is_empty() {
        return Err(SparkClientError::Config("Empty mnemonic".to_string()));
    }
    if config.postgres_connection_string.trim().is_empty() {
        return Err(SparkClientError::Config(
            "Empty postgres connection string".to_string(),
        ));
    }
    if config.token_identifier.trim().is_empty() {
        return Err(SparkClientError::Config(
            "Empty token_identifier".to_string(),
        ));
    }
    if config.page_size == 0 {
        return Err(SparkClientError::Config(
            "page_size must be greater than 0".to_string(),
        ));
    }
    Ok(())
}

fn has_more_for_page_len(page_len: usize, limit: u32) -> bool {
    page_len == limit as usize
}

fn map_payment(
    payment: Payment,
    absolute_offset: u32,
    configured_token_identifier: &str,
) -> Result<Option<SparkTransaction>, SparkClientError> {
    let details = match payment.details {
        Some(details) => details,
        None => return Ok(None),
    };

    let (token_identifier, tx_type) = match details {
        PaymentDetails::Token {
            metadata, tx_type, ..
        } => {
            let tx_type = match tx_type {
                TokenTransactionType::Mint => SparkTransactionType::Mint,
                TokenTransactionType::Burn => SparkTransactionType::Burn,
                TokenTransactionType::Transfer => return Ok(None),
            };
            (metadata.identifier, tx_type)
        }
        _ => return Ok(None),
    };

    if token_identifier != configured_token_identifier {
        return Ok(None);
    }

    if payment.status != PaymentStatus::Completed {
        return Ok(None);
    }

    let created_at = timestamp_to_datetime(payment.timestamp)?;

    Ok(Some(SparkTransaction {
        id: payment.id,
        cursor: encode_offset_cursor(absolute_offset),
        created_at,
        amount: payment.amount,
        status: payment.status,
        payment_type: payment.payment_type,
        tx_type,
        token_identifier,
    }))
}

fn timestamp_to_datetime(timestamp: u64) -> Result<DateTime<Utc>, SparkClientError> {
    let timestamp =
        i64::try_from(timestamp).map_err(|_| SparkClientError::InvalidTimestamp(timestamp))?;
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .ok_or(SparkClientError::InvalidTimestamp(timestamp as u64))
}

fn start_offset_from_cursor(cursor: Option<TxCursor>) -> Result<u32, SparkClientError> {
    match cursor {
        None => Ok(0),
        Some(cursor) => {
            let decoded = decode_offset_cursor(cursor)?;
            decoded
                .offset
                .checked_add(1)
                .ok_or_else(|| SparkClientError::Cursor("Cursor offset overflow".to_string()))
        }
    }
}

fn encode_offset_cursor(offset: u32) -> TxCursor {
    let payload = CursorPayload { offset };
    let bytes = serde_json::to_vec(&payload).expect("cursor payload serialization is infallible");
    TxCursor(BASE64URL_NOPAD.encode(&bytes))
}

fn decode_offset_cursor(cursor: TxCursor) -> Result<CursorPayload, SparkClientError> {
    let bytes = BASE64URL_NOPAD.decode(cursor.0.as_bytes())?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CursorPayload {
    offset: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use breez_sdk_spark::{PaymentMethod, PaymentType, TokenMetadata};

    #[test]
    fn cursor_roundtrip() {
        let cursor = encode_offset_cursor(123);
        let decoded = decode_offset_cursor(cursor).expect("cursor should decode");
        assert_eq!(decoded.offset, 123);
    }

    #[test]
    fn start_offset_uses_after_semantics() {
        assert_eq!(start_offset_from_cursor(None).expect("valid"), 0);
        let cursor = encode_offset_cursor(5);
        assert_eq!(start_offset_from_cursor(Some(cursor)).expect("valid"), 6);
    }

    #[test]
    fn has_more_behavior_matches_page_size() {
        assert!(has_more_for_page_len(50, 50));
        assert!(!has_more_for_page_len(49, 50));
    }

    #[test]
    fn map_payment_only_keeps_mint_and_burn_for_token_identifier() {
        let mint = sample_token_payment("id-1", "token-a", TokenTransactionType::Mint, 1000);
        let burn = sample_token_payment("id-2", "token-a", TokenTransactionType::Burn, 1001);
        let transfer =
            sample_token_payment("id-3", "token-a", TokenTransactionType::Transfer, 1002);
        let wrong_token = sample_token_payment("id-4", "token-b", TokenTransactionType::Mint, 1003);

        assert!(map_payment(mint, 0, "token-a").expect("map ok").is_some());
        assert!(map_payment(burn, 1, "token-a").expect("map ok").is_some());
        assert!(map_payment(transfer, 2, "token-a")
            .expect("map ok")
            .is_none());
        assert!(map_payment(wrong_token, 3, "token-a")
            .expect("map ok")
            .is_none());
    }

    #[test]
    fn token_identifier_required_in_config_validation() {
        let mut config = sample_config();
        config.token_identifier = " ".to_string();
        let err = validate_config(&config).expect_err("must reject empty token_identifier");
        assert!(matches!(err, SparkClientError::Config(_)));
    }

    #[test]
    fn page_size_can_be_overridden() {
        let config = SparkClientConfig {
            page_size: 7,
            ..sample_config()
        };
        assert_eq!(config.page_size(), 7);
    }

    #[test]
    fn page_size_zero_is_rejected() {
        let mut config = sample_config();
        config.page_size = 0;
        let err = validate_config(&config).expect_err("must reject page_size = 0");
        assert!(matches!(err, SparkClientError::Config(_)));
    }

    fn sample_config() -> SparkClientConfig {
        SparkClientConfig {
            api_key: "api-key".to_string(),
            network: SparkNetwork::Regtest,
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                .to_string(),
            mnemonic_passphrase: None,
            postgres_connection_string: "postgres://postgres:postgres@localhost/spark".to_string(),
            token_identifier: "token-a".to_string(),
            page_size: 100,
        }
    }

    fn sample_token_payment(
        id: &str,
        token_identifier: &str,
        tx_type: TokenTransactionType,
        timestamp: u64,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 42,
            fees: 0,
            timestamp,
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: TokenMetadata {
                    identifier: token_identifier.to_string(),
                    issuer_public_key: "02deadbeef".to_string(),
                    name: "Token".to_string(),
                    ticker: "TOK".to_string(),
                    decimals: 8,
                    max_supply: 1_000_000,
                    is_freezable: false,
                },
                tx_hash: "tx-hash".to_string(),
                tx_type,
                invoice_details: None,
                conversion_info: None,
            }),
            conversion_details: None,
        }
    }
}

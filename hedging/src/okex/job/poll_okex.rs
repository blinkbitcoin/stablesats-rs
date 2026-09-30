use tracing::instrument;

use okex_client::{OkexClient, OkexClientError, PositionSize};
use shared::payload::OKEX_EXCHANGE_ID;

use crate::{error::HedgingError, okex::*};

#[instrument(name = "hedging.okex.job.poll_okex", skip_all)]
pub async fn execute(
    pool: &sqlx::PgPool,
    okex_orders: OkexOrders,
    okex_transfers: OkexTransfers,
    okex: OkexClient,
    funding_config: OkexFundingConfig,
    ledger: &ledger::Ledger,
) -> Result<(), HedgingError> {
    let PositionSize {
        usd_cents,
        instrument_id,
        ..
    } = okex.get_position_in_signed_usd_cents().await?;
    let tx = pool.begin().await?;

    ledger
        .adjust_okex_position(
            tx,
            usd_cents,
            OKEX_EXCHANGE_ID.to_string(),
            instrument_id.to_string(),
        )
        .await?;

    let mut execute_sweep = false;
    for id in okex_orders.open_orders().await? {
        match okex.order_details(id.clone()).await {
            Ok(details) => {
                okex_orders.update_order(details).await?;
            }
            Err(OkexClientError::OrderDoesNotExist)
            | Err(OkexClientError::ParameterClientIdNotFound) => {
                okex_orders.mark_as_lost(id).await?;
                execute_sweep = true;
            }
            Err(res) => return Err(res.into()),
        }
    }

    if execute_sweep {
        okex_orders.sweep_lost_records().await?;
    }

    let mut execute_transfer_sweep = false;
    for id in okex_transfers.get_pending_transfers().await? {
        match okex.transfer_state_by_client_id(id.clone()).await {
            Ok(details) => {
                okex_transfers.update_transfer(details).await?;
            }
            Err(OkexClientError::ParameterClientIdError)
            | Err(OkexClientError::ParameterClientIdNotFound) => {
                okex_transfers.mark_as_lost(id).await?;
                execute_transfer_sweep = true;
            }
            Err(res) => return Err(res.into()),
        }
    }

    for (id, address, amount, created_at) in okex_transfers.get_pending_deposits().await? {
        match okex.fetch_deposit(address, amount).await {
            Ok(details) => {
                okex_transfers
                    .update_deposit(id, details.state, details.transaction_id)
                    .await?;
            }
            Err(OkexClientError::UnexpectedResponse { .. }) => {
                if chrono::Utc::now() - created_at > funding_config.deposit_lost_timeout_seconds {
                    okex_transfers.mark_as_lost(id).await?;
                    execute_transfer_sweep = true;
                }
            }
            Err(res) => return Err(res.into()),
        }
    }

    for id in okex_transfers.get_pending_withdrawals().await? {
        match okex.fetch_withdrawal_by_client_id(id.clone()).await {
            Ok(details) => {
                okex_transfers.update_withdrawal(details).await?;
            }
            Err(OkexClientError::WithdrawalIdDoesNotExist)
            | Err(OkexClientError::ParameterClientIdError)
            | Err(OkexClientError::ParameterClientIdNotFound) => {
                okex_transfers.mark_as_lost(id).await?;
                execute_transfer_sweep = true;
            }
            Err(res) => return Err(res.into()),
        }
    }

    if execute_transfer_sweep {
        okex_transfers.sweep_lost_records().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[sqlx::test(migrations = "../migrations")]
    async fn reconciles_nonempty_funding_history(pool: sqlx::PgPool) -> anyhow::Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let pool = &pool;
            let ledger = ledger::Ledger::init(pool).await?;
            let orders = OkexOrders::new(pool.clone()).await?;
            let transfers = OkexTransfers::new(pool.clone()).await?;
            let exchange = okex_client::test_support::Exchange::start().await;
            let client = exchange.client().await?;
            for action in ["deposit", "withdraw"] {
                let shared = TransferReservationSharedData {
                    correlation_id: uuid::Uuid::new_v4().into(), action_type: action.into(), action_unit: "BTC".into(),
                    target_usd_exposure: dec!(0), current_usd_exposure: dec!(0), trading_btc_used_balance: dec!(0),
                    trading_btc_total_balance: dec!(0), current_usd_btc_price: dec!(50000), funding_btc_total_balance: dec!(1),
                };
                let id = transfers.reserve_transfer_slot(TransferReservation {
                    action_size: Some(dec!(0.01)), fee: dec!(0), transfer_from: "source".into(), transfer_to: "address".into(), shared: &shared,
                }).await?.unwrap();
                let id_string = String::from(id);
                for (exchange_state, expected) in [("0", "pending"), ("2", "success")] {
                    let (path, data) = if action == "deposit" {
                        ("/api/v5/asset/deposit-history".to_string(), json!({"actualDepBlkConfirm":"1","amt":"0.01","ccy":"BTC","chain":"BTC-Bitcoin","depId":"1","from":"source","state":exchange_state,"to":"address","ts":"0","txId":"deposit-tx"}))
                    } else {
                        (format!("/api/v5/asset/withdrawal-history?ccy=BTC&clientId={id_string}"), json!({"ccy":"BTC","chain":"BTC-Bitcoin","amt":"0.01","ts":"0","from":"source","to":"address","txId":"withdrawal-tx","state":exchange_state,"wdId":"1","clientId":id_string}))
                    };
                    exchange.reply_once("GET", &path, okex_client::test_support::StatusCode::OK, json!({"code":"0","msg":"","data":[data]})).await;
                    execute(pool, orders.clone(), transfers.clone(), client.clone(), OkexFundingConfig::default(), &ledger).await?;
                    let row: (String, Option<String>) = sqlx::query_as("SELECT state, transfer_id FROM okex_transfers WHERE client_transfer_id = $1").bind(&id_string).fetch_one(pool).await?;
                    assert_eq!(row.0, expected);
                    assert_eq!(row.1.as_deref(), Some(if action == "deposit" { "deposit-tx" } else { "withdrawal-tx" }));
                }
            }
            anyhow::Ok(())
        }).await?
    }
}

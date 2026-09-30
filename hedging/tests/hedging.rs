use galoy_client::GaloyClientConfig;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serial_test::{file_serial, serial};

use std::env;

use bria_client::*;
use ledger::*;
use okex_client::*;
use shared::pubsub::*;

use hedging::*;
use shared::test_utils::DatabaseTestFixture;

use okex_client::test_support as support;

fn galoy_client_config() -> GaloyClientConfig {
    let api = env::var("GALOY_GRAPHQL_URI").expect("GALOY_GRAPHQL_URI not set");
    let api_key = env::var("GALOY_API_KEY").expect("GALOY_API_KEY not set");

    GaloyClientConfig { api, api_key }
}

fn bria_client_config() -> BriaClientConfig {
    let url = env::var("BRIA_URL").unwrap_or("http://localhost:2742".to_string());
    let profile_api_key = "bria_dev_000000000000000000000".to_string();
    let wallet_name = "dev-wallet".to_string();
    let payout_queue_name = "dev-queue".to_string();
    let onchain_address_external_id = "stablesats_external_id".to_string();

    BriaClientConfig {
        url,
        profile_api_key,
        wallet_name,
        onchain_address_external_id,
        payout_queue_name,
    }
}

#[tokio::test]
#[serial]
#[file_serial]
async fn hedging() -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(120), run_hedging()).await?
}

async fn wait_for_position(
    events: &mut tokio::sync::broadcast::Receiver<LedgerEvent>,
    expected: Decimal,
) -> anyhow::Result<()> {
    loop {
        if let LedgerEventData::BalanceUpdated(balance) = events.recv().await?.data {
            if balance.settled_cr_balance - balance.settled_dr_balance == expected {
                return Ok(());
            }
        }
    }
}

async fn run_hedging() -> anyhow::Result<()> {
    let exchange = support::Exchange::start().await;
    let exchange_config = exchange.config();
    let db_fixture = DatabaseTestFixture::new().await?;
    let pool = db_fixture.pool().clone();
    let ledger = Ledger::init(&pool).await?;
    let (_, health) = futures::channel::mpsc::unbounded();
    let (_, ticks) = memory::channel(chrono::Duration::seconds(1));
    let _app = HedgingApp::run(
        pool.clone(),
        health,
        HedgingAppConfig::default(),
        OkexConfig {
            client: exchange_config.clone(),
            poll_frequency: std::time::Duration::from_secs(1),
            ..Default::default()
        },
        galoy_client_config(),
        bria_client_config(),
        ticks,
        ledger.clone(),
    )
    .await?;
    let okex = OkexClient::new(exchange_config).await?;
    let mut events = ledger.usd_okex_position_balance_events().await?;

    ledger
        .user_buys_usd(
            pool.begin().await?,
            LedgerTxId::new(),
            UserBuysUsdParams {
                satoshi_amount: dec!(1000000),
                usd_cents_amount: dec!(50000),
                meta: UserBuysUsdMeta {
                    timestamp: chrono::Utc::now(),
                    btc_tx_id: "btc_tx_id".into(),
                    usd_tx_id: "usd_tx_id".into(),
                },
            },
        )
        .await?;
    wait_for_position(&mut events, dec!(-500)).await?;
    assert_eq!(
        okex.get_position_in_signed_usd_cents().await?.usd_cents,
        dec!(-50000)
    );

    // Observe the close and re-hedge through ordered ledger events. Polling the
    // exchange after a sleep can miss the zero position once hedging repairs it.
    okex.close_positions(ClientOrderId::new()).await?;
    wait_for_position(&mut events, dec!(0)).await?;
    wait_for_position(&mut events, dec!(-500)).await?;

    ledger
        .user_sells_usd(
            pool.begin().await?,
            LedgerTxId::new(),
            UserSellsUsdParams {
                satoshi_amount: dec!(1000000),
                usd_cents_amount: dec!(50000),
                meta: UserSellsUsdMeta {
                    timestamp: chrono::Utc::now(),
                    btc_tx_id: "btc_tx_id".into(),
                    usd_tx_id: "usd_tx_id".into(),
                },
            },
        )
        .await?;
    wait_for_position(&mut events, dec!(0)).await?;
    assert_eq!(
        okex.get_position_in_signed_usd_cents().await?.usd_cents,
        dec!(0)
    );
    Ok(())
}

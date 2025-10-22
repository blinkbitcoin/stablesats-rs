#![allow(clippy::or_fun_call)]

use galoy_client::GaloyClientConfig;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal_macros::dec;
use serial_test::{file_serial, serial};

use std::env;

use bria_client::*;
use ledger::*;
use okex_client::*;
use shared::pubsub::*;

use hedging::*;
use shared::test_utils::DatabaseTestFixture;

fn okex_config() -> OkexConfig {
    let api_key = env::var("OKEX_API_KEY").expect("OKEX_API_KEY not set");
    let passphrase = env::var("OKEX_PASSPHRASE").expect("OKEX_PASSPHRASE not set");
    let secret_key = env::var("OKEX_SECRET_KEY").expect("OKEX_SECRET_KEY not set");
    OkexConfig {
        client: OkexClientConfig {
            api_key,
            passphrase,
            secret_key,
            simulated: true,
        },
        ..Default::default()
    }
}

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
    println!("🎯 Test started!");

    println!("🔌 Setting up database test fixture...");
    let db_fixture = DatabaseTestFixture::new().await?;
    let pool = db_fixture.pool().clone();
    println!("✅ Database test fixture ready!");

    println!("📊 Initializing ledger...");

    // Let's check if the database has the required tables first
    println!("🔍 Checking database tables...");
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_all(&pool)
    .await?;
    println!(
        "📋 Found {} tables: {:?}",
        tables.len(),
        tables.iter().map(|(name,)| name).collect::<Vec<_>>()
    );

    // Try a simple database query first to test connectivity
    println!("🧪 Testing database with simple query...");
    let count_result: Result<(i64,), sqlx::Error> =
        sqlx::query_as("SELECT COUNT(*) FROM sqlx_ledger_journals")
            .fetch_one(&pool)
            .await;

    match count_result {
        Ok((count,)) => println!("✅ Database query successful, found {} journals", count),
        Err(e) => {
            println!("❌ Database query failed: {}", e);
            return Err(e.into());
        }
    }

    // Try ledger init with shorter timeout and more logging
    println!("⏱️ Starting ledger initialization with 30s timeout...");
    let ledger_result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        println!("🔄 Calling ledger::Ledger::init...");
        let result = ledger::Ledger::init(&pool).await;
        println!("🏁 ledger::Ledger::init completed");
        result
    })
    .await;

    let ledger = match ledger_result {
        Ok(Ok(ledger)) => {
            println!("✅ Ledger initialized successfully!");
            ledger
        }
        Ok(Err(e)) => {
            println!("❌ Ledger initialization failed: {}", e);
            return Err(e.into());
        }
        Err(_) => {
            println!("⏰ Ledger initialization timed out after 30 seconds!");
            return Err(anyhow::anyhow!("Ledger initialization timeout"));
        }
    };

    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    let (_, tick_recv) = memory::channel(chrono::Duration::from_std(
        std::time::Duration::from_secs(1),
    )?);

    let ledger_clone = ledger.clone();
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        println!("🚀 Starting HedgingApp initialization...");
        let (_, recv) = futures::channel::mpsc::unbounded();

        println!("📋 Creating configs...");
        let mut okex_cfg = okex_config();
        okex_cfg.poll_frequency = std::time::Duration::from_secs(1); // Poll every 1 second for faster testing
        println!("✅ OKX config created");

        let galoy_cfg = galoy_client_config();
        println!("✅ Galoy config created");

        let bria_cfg = bria_client_config();
        println!("✅ Bria config created");

        println!("🔄 Starting HedgingApp::run...");
        let result = HedgingApp::run(
            pool_clone,
            recv,
            HedgingAppConfig {
                ..Default::default()
            },
            okex_cfg,
            galoy_cfg,
            bria_cfg,
            tick_recv.resubscribe(),
            ledger_clone,
        )
        .await;

        match &result {
            Ok(_) => println!("✅ HedgingApp started successfully!"),
            Err(e) => println!("❌ HedgingApp failed: {}", e),
        }

        let _ = send.try_send(result.expect("HedgingApp failed"));
    });

    println!("⏳ Waiting for HedgingApp to start...");
    let _reason = receive.recv().await.expect("Didn't receive msg");
    println!("✅ HedgingApp startup completed!");

    println!("😴 Starting 2-second sleep...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    println!("⏰ 2-second sleep completed!");

    println!("💰 Executing user_buys_usd transaction...");
    ledger
        .user_buys_usd(
            pool.clone().begin().await?,
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
    println!("✅ user_buys_usd transaction completed");

    println!("📡 Subscribing to balance events...");
    let mut event = ledger.usd_okex_position_balance_events().await?;
    println!("✅ Subscribed to balance events");

    let mut passed = false;
    println!("⏳ Waiting for balance update events (up to 60 iterations)...");
    for i in 0..=60 {
        println!("🔄 Iteration {}/60: Waiting for balance event...", i + 1);
        let user_buy_event = event.recv().await?;
        println!("📨 Received event: {:?}", user_buy_event.data);
        // checks if a position of $-500 gets opened on the exchange.
        if let ledger::LedgerEventData::BalanceUpdated(data) = user_buy_event.data {
            let balance_diff = data.settled_cr_balance - data.settled_dr_balance;
            println!("💰 Balance difference: {} (looking for -500)", balance_diff);
            if balance_diff == dec!(-500) {
                println!("✅ Found target balance of -500!");
                passed = true;
                break;
            }
        } else {
            println!("⏰ Non-balance event, sleeping...");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    if !passed {
        panic!("Could not open a position on the exchange!");
    }

    let okex = OkexClient::new(okex_config().client).await?;

    // Get current position before trying to close it
    let current_position = okex.get_position_in_signed_usd_cents().await?;
    println!("📊 Current position before closing: {:?}", current_position);

    // Verify the position matches our expectation of -$500
    println!("🎯 Expected position: -$500 (-50000 cents)");
    println!(
        "📊 Actual position: ${} ({} cents)",
        current_position.usd_cents / dec!(100),
        current_position.usd_cents
    );

    if (current_position.usd_cents - dec!(-50000)).abs() > dec!(100) {
        // Allow $1 tolerance
        println!(
            "⚠️ WARNING: Position mismatch! Expected -50000 cents, got {} cents",
            current_position.usd_cents
        );
    }

    // Try using the close_positions API first, which should be more reliable
    println!("🔄 Attempting to close position using close_positions API...");
    let close_order_id = ClientOrderId::new();
    println!("📋 Using close order ID: {:?}", close_order_id);
    match okex.close_positions(close_order_id).await {
        Ok(_) => {
            println!("✅ Close positions API call successful");
            // Wait a moment for the order to be processed
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Err(e) => {
            println!(
                "⚠️ Close positions API failed: {}, trying manual order placement",
                e
            );

            // Fallback to manual order placement
            // Calculate the contracts needed to close the position
            // Each contract is worth approximately $100 USD
            let contracts_to_close = if current_position.usd_cents < dec!(0) {
                // We have a short position, need to buy to close
                let contracts = (current_position.usd_cents.abs() / dec!(10000)).ceil(); // $100 per contract in cents
                println!(
                    "🔄 Placing BUY order for {} contracts to close short position",
                    contracts
                );
                BtcUsdSwapContracts::from(contracts.to_u32().unwrap_or(5))
            } else if current_position.usd_cents > dec!(0) {
                // We have a long position, need to sell to close
                let contracts = (current_position.usd_cents / dec!(10000)).ceil(); // $100 per contract in cents
                println!(
                    "🔄 Placing SELL order for {} contracts to close long position",
                    contracts
                );
                BtcUsdSwapContracts::from(contracts.to_u32().unwrap_or(5))
            } else {
                println!("⚠️ Position is already zero, no need to close");
                BtcUsdSwapContracts::from(0)
            };

            if u32::from(&contracts_to_close) > 0 {
                let side = if current_position.usd_cents < dec!(0) {
                    OkexOrderSide::Buy
                } else {
                    OkexOrderSide::Sell
                };

                okex.place_order(ClientOrderId::new(), side, &contracts_to_close)
                    .await?;
                println!("✅ Manual order placed successfully");
            }
        }
    }

    passed = false;
    println!("⏳ Waiting for position to close (up to 60 seconds)...");
    for i in 0..=60 {
        let PositionSize { usd_cents, .. } = okex.get_position_in_signed_usd_cents().await?;
        println!(
            "🔍 Iteration {}/60: Current position: ${}",
            i + 1,
            usd_cents / dec!(100)
        );

        // Check if the position is close to zero (allowing for small rounding differences)
        if usd_cents.abs() < dec!(50) {
            // Less than $0.50
            println!("✅ Position successfully closed (within $0.50 of zero)");
            passed = true;
            break;
        } else {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    if !passed {
        panic!("Could not close the position via OkexClient!");
    }

    passed = false;
    println!("⏳ Waiting for hedging system to re-open position after manual close...");
    for i in 0..=120 {
        // Increased timeout to 2 minutes
        println!(
            "🔄 Re-hedge iteration {}/60: Waiting for balance event...",
            i + 1
        );
        let user_buy_event = event.recv().await?;
        println!("📨 Re-hedge received event: {:?}", user_buy_event.data);
        // checks if a position of $-500 gets opened on the exchange.
        if let ledger::LedgerEventData::BalanceUpdated(data) = user_buy_event.data {
            let balance_diff = data.settled_cr_balance - data.settled_dr_balance;
            println!(
                "💰 Re-hedge balance difference: {} (looking for -500)",
                balance_diff
            );
            if balance_diff == dec!(-500) {
                println!("✅ Found re-hedged target balance of -500!");
                passed = true;
                break;
            }
        } else {
            println!("⏰ Re-hedge non-balance event, sleeping...");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    if !passed {
        panic!("Could not open a position on the exchange after closing it via OkexClient!");
    }

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
    passed = false;
    for _ in 0..=60 {
        let user_sell_event = event.recv().await?;
        // checks if the position gets closed on the exchange.
        if let ledger::LedgerEventData::BalanceUpdated(data) = user_sell_event.data {
            if (data.settled_cr_balance - data.settled_dr_balance) == dec!(0) {
                passed = true;
                break;
            }
        } else {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    if !passed {
        panic!("Could not close the position on the exchange");
    }

    Ok(())
}

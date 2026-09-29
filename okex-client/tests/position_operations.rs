use okex_client::*;
use rust_decimal_macros::dec;
use serial_test::serial;

mod support;

#[tokio::test]
#[serial]
async fn test_open_and_close_position() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 Testing OKX position open and close operations");

    let exchange = support::Exchange::start().await;
    let okex_cfg = exchange.config();
    let okex = OkexClient::new(okex_cfg).await?;

    // Step 1: Get initial position
    let initial_position = okex.get_position_in_signed_usd_cents().await?;
    assert_eq!(initial_position.usd_cents, dec!(0));
    println!("📊 Initial position: {:?}", initial_position);

    // Step 2: Open a position by placing a SELL order (creates short position)
    println!("🔄 Opening position with SELL order for 1 contract...");
    let open_order_id = ClientOrderId::new();
    okex.place_order(
        open_order_id.clone(),
        OkexOrderSide::Sell,
        &BtcUsdSwapContracts::from(1),
    )
    .await?;
    println!("✅ SELL order placed successfully");
    let details = okex.order_details(open_order_id).await?;
    assert!(details.complete);
    assert_eq!(details.sz, dec!(1));

    // Step 3: Wait for position to be established
    println!("⏳ Waiting for position to be established...");
    let mut position_established = false;
    for i in 1..=30 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let current_position = okex.get_position_in_signed_usd_cents().await?;
        println!(
            "🔍 Check {}/30: Position = ${}",
            i,
            current_position.usd_cents / dec!(100)
        );

        // Check if we have a short position (negative value)
        if current_position.usd_cents < dec!(-50) {
            assert_eq!(current_position.usd_cents, dec!(-10000));
            // Less than -$0.50
            println!(
                "✅ Position established: ${}",
                current_position.usd_cents / dec!(100)
            );
            position_established = true;
            break;
        }
    }

    if !position_established {
        return Err("Failed to establish position after placing SELL order".into());
    }

    // Step 4: Close the position using close_positions API
    println!("🔄 Closing position using close_positions API...");
    let close_order_id = ClientOrderId::new();
    okex.close_positions(close_order_id.clone()).await?;
    let details = okex.order_details(close_order_id).await?;
    assert!(details.complete);
    assert_eq!(details.sz, dec!(1));
    println!("✅ Close positions API call successful");

    // Step 5: Wait for position to be closed
    println!("⏳ Waiting for position to be closed...");
    let mut position_closed = false;
    for i in 1..=60 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let current_position = okex.get_position_in_signed_usd_cents().await?;
        println!(
            "🔍 Check {}/60: Position = ${}",
            i,
            current_position.usd_cents / dec!(100)
        );

        // Check if position is close to zero
        if current_position.usd_cents.abs() < dec!(50) {
            // Less than $0.50 in absolute value
            println!(
                "✅ Position successfully closed: ${}",
                current_position.usd_cents / dec!(100)
            );
            position_closed = true;
            break;
        }
    }

    if !position_closed {
        return Err("Failed to close position using close_positions API".into());
    }

    println!("🎉 Test completed successfully - OKX position operations working correctly");
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_manual_position_close() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 Testing manual OKX position close with opposite order");

    let exchange = support::Exchange::start().await;
    let okex_cfg = exchange.config();
    let okex = OkexClient::new(okex_cfg).await?;

    // Step 1: Open a position by placing a SELL order
    println!("🔄 Opening position with SELL order for 2 contracts...");
    let open_order_id = ClientOrderId::new();
    okex.place_order(
        open_order_id,
        OkexOrderSide::Sell,
        &BtcUsdSwapContracts::from(2),
    )
    .await?;

    // Step 2: Wait for position to be established
    println!("⏳ Waiting for position to be established...");
    let mut established_position = None;
    for i in 1..=30 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let current_position = okex.get_position_in_signed_usd_cents().await?;
        println!(
            "🔍 Check {}/30: Position = ${}",
            i,
            current_position.usd_cents / dec!(100)
        );

        if current_position.usd_cents < dec!(-100) {
            // Less than -$1.00
            established_position = Some(current_position);
            break;
        }
    }

    let position = established_position.ok_or("Failed to establish position")?;
    assert_eq!(position.usd_cents, dec!(-20000));
    println!(
        "✅ Position established: ${}",
        position.usd_cents / dec!(100)
    );

    // Step 3: Close manually with opposite BUY order
    println!("🔄 Closing position manually with BUY order for 2 contracts...");
    let close_order_id = ClientOrderId::new();
    okex.place_order(
        close_order_id,
        OkexOrderSide::Buy,
        &BtcUsdSwapContracts::from(2),
    )
    .await?;

    // Step 4: Wait for position to be closed
    println!("⏳ Waiting for position to be closed...");
    let mut position_closed = false;
    for i in 1..=60 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let current_position = okex.get_position_in_signed_usd_cents().await?;
        println!(
            "🔍 Check {}/60: Position = ${}",
            i,
            current_position.usd_cents / dec!(100)
        );

        if current_position.usd_cents.abs() < dec!(50) {
            println!(
                "✅ Position successfully closed: ${}",
                current_position.usd_cents / dec!(100)
            );
            position_closed = true;
            break;
        }
    }

    if !position_closed {
        return Err("Failed to close position with manual BUY order".into());
    }

    println!("🎉 Manual position close test completed successfully");
    Ok(())
}

#[tokio::test]
async fn transfers_collateral_between_accounts() -> anyhow::Result<()> {
    let exchange = support::Exchange::start().await;
    let okex = OkexClient::new(exchange.config()).await?;
    okex.check_leverage(dec!(4)).await?;
    assert_eq!(
        okex.get_last_price_in_usd_cents().await?.usd_cents,
        dec!(5000000)
    );
    assert_eq!(okex.get_onchain_fees().await?.min_fee, dec!(0.0002));
    assert_eq!(
        okex.funding_account_balance().await?.total_amt_in_btc,
        dec!(1)
    );
    assert_eq!(
        okex.trading_account_balance().await?.total_amt_in_btc,
        dec!(0.01)
    );

    let id = ClientTransferId::new();
    okex.transfer_funding_to_trading(id.clone(), dec!(0.02))
        .await?;
    assert_eq!(okex.transfer_state_by_client_id(id).await?.state, "success");
    assert_eq!(
        okex.trading_account_balance().await?.total_amt_in_btc,
        dec!(0.03)
    );
    assert_eq!(
        okex.funding_account_balance().await?.total_amt_in_btc,
        dec!(0.98)
    );

    let id = ClientTransferId::new();
    okex.transfer_trading_to_funding(id.clone(), dec!(0.02))
        .await?;
    assert_eq!(okex.transfer_state_by_client_id(id).await?.state, "success");
    assert_eq!(
        okex.trading_account_balance().await?.total_amt_in_btc,
        dec!(0.01)
    );
    assert_eq!(
        okex.funding_account_balance().await?.total_amt_in_btc,
        dec!(1)
    );
    Ok(())
}

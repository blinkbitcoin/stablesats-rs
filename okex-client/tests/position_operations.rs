use okex_client::*;
use rust_decimal_macros::dec;
use serial_test::serial;

fn okex_client_config() -> OkexClientConfig {
    OkexClientConfig {
        api_key: std::env::var("OKEX_API_KEY").expect("OKEX_API_KEY must be set"),
        secret_key: std::env::var("OKEX_SECRET_KEY").expect("OKEX_SECRET_KEY must be set"),
        passphrase: std::env::var("OKEX_PASSPHRASE").expect("OKEX_PASSPHRASE must be set"),
        simulated: true,
    }
}

/// Closes any existing BTC-USD-SWAP positions and waits until the position is flat.
/// This prevents "Insufficient BTC margin" errors caused by leftover positions
/// from prior failed test runs.
async fn ensure_no_open_positions(okex: &OkexClient) -> Result<(), Box<dyn std::error::Error>> {
    let position = okex.get_position_in_signed_usd_cents().await?;
    if position.usd_cents.abs() < dec!(50) {
        println!("✅ No existing position to clean up");
        return Ok(());
    }

    println!(
        "⚠️  Found existing position: ${}, closing...",
        position.usd_cents / dec!(100)
    );
    let close_id = ClientOrderId::new();
    okex.close_positions(close_id).await?;

    for i in 1..=60 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let current = okex.get_position_in_signed_usd_cents().await?;
        if current.usd_cents.abs() < dec!(50) {
            println!("✅ Existing position closed after {i}s");
            return Ok(());
        }
    }

    Err("Failed to close existing position within 60s".into())
}

#[tokio::test]
#[serial]
async fn test_open_and_close_position() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 Testing OKX position open and close operations");

    let okex_cfg = okex_client_config();
    let okex = OkexClient::new(okex_cfg).await?;

    // Step 0: Clean up any leftover positions from prior runs
    ensure_no_open_positions(&okex).await?;

    // Step 1: Get initial position
    let initial_position = okex.get_position_in_signed_usd_cents().await?;
    println!("📊 Initial position: {:?}", initial_position);

    // Step 2: Open a position by placing a SELL order (creates short position)
    println!("🔄 Opening position with SELL order for 1 contract...");
    let open_order_id = ClientOrderId::new();
    okex.place_order(
        open_order_id,
        OkexOrderSide::Sell,
        &BtcUsdSwapContracts::from(1),
    )
    .await?;
    println!("✅ SELL order placed successfully");

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
    okex.close_positions(close_order_id).await?;
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

    let okex_cfg = okex_client_config();
    let okex = OkexClient::new(okex_cfg).await?;

    // Step 0: Clean up any leftover positions from prior runs
    ensure_no_open_positions(&okex).await?;

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

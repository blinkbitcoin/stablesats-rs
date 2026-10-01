use anyhow::Context;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serial_test::serial;

use std::env;

use okex_client::*;

async fn configured_okex_client() -> anyhow::Result<OkexClient> {
    let api_key = env::var("OKEX_API_KEY").expect("OKEX_API_KEY not set");
    let passphrase = env::var("OKEX_PASSPHRASE").expect("OKEX_PASSPHRASE not set");
    let secret_key = env::var("OKEX_SECRET_KEY").expect("OKEX_SECRET_KEY not set");

    let client = OkexClient::new(OkexClientConfig {
        api_key,
        passphrase,
        secret_key,
        simulated: true,
    })
    .await?;

    Ok(client)
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn get_deposit_address_data() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let address = client.get_funding_deposit_address().await?;
    assert!(address.value.len() > 10);

    Ok(())
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn get_onchain_fees_data() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let fees = client.get_onchain_fees().await?;
    assert_eq!(fees.ccy, "BTC".to_string());
    assert_eq!(fees.chain, "BTC-Bitcoin".to_string());
    assert!(fees.min_fee >= Decimal::ZERO && fees.min_fee < Decimal::ONE);
    assert!(fees.max_fee >= Decimal::ZERO && fees.max_fee < Decimal::ONE);
    assert!(fees.min_withdraw >= Decimal::ZERO && fees.min_withdraw < Decimal::ONE);
    assert!(fees.max_withdraw >= Decimal::ZERO && fees.max_withdraw > Decimal::ONE);

    Ok(())
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn client_is_missing_header() -> anyhow::Result<()> {
    let client = OkexClient::new(OkexClientConfig {
        api_key: "".to_string(),
        passphrase: "".to_string(),
        secret_key: "".to_string(),
        simulated: true,
    })
    .await;

    assert!(client.is_err());

    if let Err(OkexClientError::UnexpectedResponse { msg, .. }) = client {
        assert!(msg.contains("header"));
    } else {
        assert!(false)
    }

    Ok(())
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn funding_account_balance() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let avail_balance = client.funding_account_balance().await?;
    let balance = avail_balance.total_amt_in_btc;
    let minimum_balance = dec!(0);
    assert!(balance >= minimum_balance);

    Ok(())
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn trading_account_balance() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let avail_balance = client.trading_account_balance().await?;
    let minimum_balance = dec!(0);
    assert!(avail_balance.total_amt_in_btc >= minimum_balance);

    Ok(())
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn unknown_client_order_id() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let id = ClientOrderId::new();
    let result = client.order_details(id).await;
    if let Err(OkexClientError::OrderDoesNotExist) = result {
        assert!(true)
    } else {
        assert!(false)
    }
    Ok(())
}

#[tokio::test]
#[ignore = "only works against real okex client"]
async fn deposit_status() -> anyhow::Result<()> {
    if let (Ok(deposit_addr), Ok(deposit_amount)) = (
        env::var("OKEX_DEPOSIT_ADDRESS"),
        env::var("OKEX_DEPOSIT_AMOUNT"),
    ) {
        let amt = Decimal::from_str_exact(&deposit_amount)?;
        let client = configured_okex_client().await?;

        let deposit = client.fetch_deposit(deposit_addr, amt).await?;

        assert_eq!(deposit.state, "2".to_string());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "only works against real okex client"]
async fn withdraw_to_onchain_address() -> anyhow::Result<()> {
    let amount = OKEX_MINIMUM_WITHDRAWAL_AMOUNT;
    let fee = OKEX_MINIMUM_WITHDRAWAL_FEE;
    if let Ok(onchain_address) = env::var("ONCHAIN_BTC_WITHDRAWAL_ADDRESS") {
        let client = configured_okex_client().await?;
        let withdraw_id = client
            .withdraw_btc_onchain(ClientTransferId::new(), amount, fee, onchain_address)
            .await?;

        assert!(!withdraw_id.value.is_empty());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "transfer call is rate limited"]
async fn transfer_trading_to_funding() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let amount = dec!(0.00001);
    let transfer_id = client
        .transfer_trading_to_funding(ClientTransferId::new(), amount)
        .await?;

    assert!(!transfer_id.value.is_empty());

    Ok(())
}

#[tokio::test]
#[ignore = "transfer call is rate limited"]
async fn transfer_state() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let amount = dec!(0.00001);
    let client_id = ClientTransferId::new();
    let client_id_val: String = client_id.clone().into();
    let transfer_id = client
        .transfer_funding_to_trading(client_id, amount)
        .await?;

    let transfer_id_val = transfer_id.value.clone();
    let transfer_state = client.transfer_state(transfer_id).await?;

    assert_eq!(transfer_state.client_id, client_id_val);
    assert_eq!(transfer_state.transfer_id, transfer_id_val);
    assert_eq!(transfer_state.state, "success".to_string());

    Ok(())
}

#[tokio::test]
#[ignore = "transfer call is rate limited"]
async fn transfer_state_by_client_id() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;
    let amount = dec!(0.00001);
    let client_id = ClientTransferId::new();
    let client_id_val: String = client_id.clone().into();
    let transfer_id = client
        .transfer_funding_to_trading(client_id.clone(), amount)
        .await?;

    let transfer_state = client.transfer_state_by_client_id(client_id).await?;

    assert_eq!(transfer_state.client_id, client_id_val);
    assert_eq!(transfer_state.transfer_id, transfer_id.value);
    assert_eq!(transfer_state.state, "success".to_string());

    Ok(())
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn open_close_position() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;

    client.close_positions(ClientOrderId::new()).await?;

    client
        .place_order(
            ClientOrderId::new(),
            OkexOrderSide::Sell,
            &BtcUsdSwapContracts::from(1),
        )
        .await?;

    let position = client.get_position_in_signed_usd_cents().await?;

    assert!(position.usd_cents < dec!(-95));
    assert!(position.usd_cents > dec!(-105));

    assert!(client.close_positions(ClientOrderId::new()).await.is_ok());

    Ok(())
}

#[tokio::test]
#[serial]
#[ignore = "avoid rate limit"]
async fn last_price() -> anyhow::Result<()> {
    let client = configured_okex_client().await?;

    let last_price = client.get_last_price_in_usd_cents().await?;

    assert!(!last_price.usd_cents.is_zero());
    assert!(last_price.usd_cents.is_sign_positive());

    Ok(())
}

fn validate_demo_balances(
    funding: Decimal,
    trading: Decimal,
    price_cents: Decimal,
    leverage: Decimal,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        price_cents > Decimal::ZERO && leverage > Decimal::ZERO,
        "DEMO_ACCOUNT_PREFLIGHT: invalid price or leverage"
    );
    // Two funding transfers and one contract, with margin headroom and the trading transfer.
    anyhow::ensure!(
        funding >= dec!(0.00002),
        "DEMO_ACCOUNT_PREFLIGHT: funding account needs at least 0.00002 BTC available"
    );
    let required_trading = dec!(0.00001) + dec!(2) * dec!(10000) / price_cents / leverage;
    anyhow::ensure!(
        trading >= required_trading,
        "DEMO_ACCOUNT_PREFLIGHT: trading account needs at least {required_trading} BTC available"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires funded OKX demo account"]
async fn demo_account_ready() -> anyhow::Result<()> {
    let client = configured_okex_client()
        .await
        .context("DEMO_ACCOUNT_PREFLIGHT: credentials or account configuration rejected")?;
    check_demo_account(&client).await
}

async fn check_demo_account(client: &OkexClient) -> anyhow::Result<()> {
    let funding = client
        .funding_account_balance()
        .await
        .context("DEMO_ACCOUNT_PREFLIGHT: funding balance unavailable")?;
    let trading = client
        .trading_account_balance()
        .await
        .context("DEMO_ACCOUNT_PREFLIGHT: trading balance unavailable")?;
    let price = client
        .get_last_price_in_usd_cents()
        .await
        .context("DEMO_ACCOUNT_PREFLIGHT: price unavailable")?;
    let leverage = client
        .leverage_info()
        .await
        .context("DEMO_ACCOUNT_PREFLIGHT: leverage unavailable")?;
    validate_demo_balances(
        funding.free_amt_in_btc,
        trading.free_amt_in_btc,
        price.usd_cents,
        leverage.lever,
    )
}

#[test]
fn demo_balance_preflight_distinguishes_account_prerequisites() {
    let price = dec!(5000000);
    let leverage = dec!(4);
    let minimum = dec!(0.00101);
    validate_demo_balances(dec!(0.00002), minimum, price, leverage).unwrap();
    assert!(
        validate_demo_balances(dec!(0.000019), minimum, price, leverage)
            .unwrap_err()
            .to_string()
            .contains("funding account")
    );
    assert!(
        validate_demo_balances(dec!(1), minimum - dec!(0.000001), price, leverage)
            .unwrap_err()
            .to_string()
            .contains("trading account")
    );
    for (price, leverage) in [
        (dec!(0), leverage),
        (price, dec!(0)),
        (dec!(-1), leverage),
        (price, dec!(-1)),
    ] {
        assert!(validate_demo_balances(dec!(1), dec!(1), price, leverage)
            .unwrap_err()
            .to_string()
            .contains("invalid price or leverage"));
    }
}

#[tokio::test]
async fn demo_preflight_checks_real_client_responses() -> anyhow::Result<()> {
    use okex_client::test_support::{Exchange, StatusCode};
    use serde_json::json;
    let exchange = Exchange::start().await;
    let client = exchange.client().await?;
    check_demo_account(&client).await?;
    for (path, message) in [
        (
            "/api/v5/asset/balances?ccy=BTC",
            "funding balance unavailable",
        ),
        (
            "/api/v5/account/balance?ccy=BTC",
            "trading balance unavailable",
        ),
        (
            "/api/v5/market/ticker?instId=BTC-USD-SWAP",
            "price unavailable",
        ),
        (
            "/api/v5/account/leverage-info?instId=BTC-USD-SWAP&mgnMode=cross",
            "leverage unavailable",
        ),
    ] {
        exchange
            .reply_once(
                "GET",
                path,
                StatusCode::BAD_REQUEST,
                json!({"code":"rejected","msg":"test rejection","data":null}),
            )
            .await;
        let error = check_demo_account(&client).await.unwrap_err().to_string();
        assert!(
            error.contains("DEMO_ACCOUNT_PREFLIGHT") && error.contains(message),
            "{error}"
        );
    }
    exchange.assert_all_replies_consumed().await;
    Ok(())
}

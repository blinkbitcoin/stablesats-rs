use okex_client::{test_support::Exchange, *};
use rust_decimal_macros::dec;

#[tokio::test]
async fn test_open_and_close_position() -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let exchange = Exchange::start().await;
        let okex = exchange.client().await?;
        assert_eq!(
            okex.get_position_in_signed_usd_cents().await?.usd_cents,
            dec!(0)
        );
        let open_id = ClientOrderId::new();
        okex.place_order(
            open_id.clone(),
            OkexOrderSide::Sell,
            &BtcUsdSwapContracts::from(1),
        )
        .await?;
        let details = okex.order_details(open_id).await?;
        assert!(details.complete);
        assert_eq!(details.sz, dec!(1));
        assert_eq!(
            okex.get_position_in_signed_usd_cents().await?.usd_cents,
            dec!(-10000)
        );
        let close_id = ClientOrderId::new();
        okex.close_positions(close_id.clone()).await?;
        let details = okex.order_details(close_id).await?;
        assert!(details.complete);
        assert_eq!(details.sz, dec!(1));
        assert_eq!(
            okex.get_position_in_signed_usd_cents().await?.usd_cents,
            dec!(0)
        );
        // OKX reports 51023 when an already-flat position is closed.
        okex.close_positions(ClientOrderId::new()).await?;
        anyhow::Ok(())
    })
    .await?
}

#[tokio::test]
async fn test_manual_position_close() -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let exchange = Exchange::start().await;
        let okex = exchange.client().await?;
        okex.place_order(
            ClientOrderId::new(),
            OkexOrderSide::Sell,
            &BtcUsdSwapContracts::from(2),
        )
        .await?;
        assert_eq!(
            okex.get_position_in_signed_usd_cents().await?.usd_cents,
            dec!(-20000)
        );
        okex.place_order(
            ClientOrderId::new(),
            OkexOrderSide::Buy,
            &BtcUsdSwapContracts::from(2),
        )
        .await?;
        assert_eq!(
            okex.get_position_in_signed_usd_cents().await?.usd_cents,
            dec!(0)
        );
        anyhow::Ok(())
    })
    .await?
}

#[tokio::test]
async fn transfers_collateral_between_accounts() -> anyhow::Result<()> {
    tokio::time::timeout(
        std::time::Duration::from_secs(120),
        transfers_collateral_between_accounts_body(),
    )
    .await?
}

async fn transfers_collateral_between_accounts_body() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let okex = exchange.client().await?;
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

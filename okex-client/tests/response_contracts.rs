use okex_client::test_support::StatusCode;
use okex_client::{test_support::Exchange, *};
use rust_decimal_macros::dec;
use serde_json::{json, Value};

const FUNDING: &str = "/api/v5/asset/balances?ccy=BTC";
const TICKER: &str = "/api/v5/market/ticker?instId=BTC-USD-SWAP";
const DEPOSITS: &str = "/api/v5/asset/deposit-history";
const ORDER: &str = "/api/v5/trade/order";
const CLOSE: &str = "/api/v5/trade/close-position";
const POSITIONS: &str = "/api/v5/account/positions?instId=BTC-USD-SWAP";

fn error(code: &str, msg: &str) -> Value {
    json!({"code":code,"msg":msg,"data":null})
}

async fn exercise(client: &OkexClient, path: &str) -> Result<(), OkexClientError> {
    match path {
        FUNDING => {
            client.funding_account_balance().await?;
        }
        TICKER => {
            client.get_last_price_in_usd_cents().await?;
        }
        DEPOSITS => {
            // An empty successful array proves extraction completed; the subsequent lookup fails.
            assert!(
                matches!(client.fetch_deposit("address".into(), dec!(1)).await,
                Err(OkexClientError::UnexpectedResponse { code, .. }) if code == "0")
            );
        }
        ORDER => {
            client
                .place_order(
                    ClientOrderId::new(),
                    OkexOrderSide::Sell,
                    &BtcUsdSwapContracts::from(1),
                )
                .await?;
        }
        CLOSE => {
            client.close_positions(ClientOrderId::new()).await?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[tokio::test]
async fn timestamp_expiry_retries_once_for_each_response_helper() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let client = exchange.client().await?;
    for (method, path) in [
        ("GET", FUNDING),
        ("GET", TICKER),
        ("GET", DEPOSITS),
        ("POST", ORDER),
        ("POST", CLOSE),
    ] {
        exchange
            .reply_once(
                method,
                path,
                StatusCode::UNAUTHORIZED,
                error("50102", "Timestamp expired"),
            )
            .await;
        exercise(&client, path).await?;
        let bodies = exchange.request_bodies(method, path).await;
        assert_eq!(bodies.len(), 2, "{path}");
        assert_eq!(
            bodies[0], bodies[1],
            "retry must preserve the request body and client ID"
        );
    }
    // A second expiry is returned instead of an unbounded retry loop.
    for _ in 0..2 {
        exchange
            .reply_once(
                "GET",
                FUNDING,
                StatusCode::UNAUTHORIZED,
                error("50102", "Timestamp expired"),
            )
            .await;
    }
    assert!(
        matches!(client.funding_account_balance().await, Err(OkexClientError::UnexpectedResponse { code, .. }) if code == "50102")
    );
    assert_eq!(exchange.request_bodies("GET", FUNDING).await.len(), 4);
    exchange.assert_all_replies_consumed().await;
    Ok(())
}

#[tokio::test]
async fn non_success_responses_preserve_exchange_errors_and_do_not_retry() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let client = exchange.client().await?;
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::BAD_REQUEST,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        exchange
            .reply_once(
                "GET",
                FUNDING,
                status,
                error("rejected", "diagnostic from exchange"),
            )
            .await;
        assert!(matches!(client.funding_account_balance().await,
            Err(OkexClientError::UnexpectedResponse { code, msg }) if code == "rejected" && msg == "diagnostic from exchange"));
    }
    assert_eq!(exchange.request_bodies("GET", FUNDING).await.len(), 3);
    exchange
        .reply_raw_once(
            "GET",
            FUNDING,
            StatusCode::BAD_GATEWAY,
            "upstream unavailable".into(),
        )
        .await;
    assert!(matches!(
        client.funding_account_balance().await,
        Err(OkexClientError::Deserialization(_))
    ));
    assert!(matches!(
        client.order_details(ClientOrderId::new()).await,
        Err(OkexClientError::OrderDoesNotExist)
    ));
    assert!(matches!(
        client
            .transfer_state_by_client_id(ClientTransferId::new())
            .await,
        Err(OkexClientError::ParameterClientIdNotFound)
    ));
    assert!(matches!(
        client
            .transfer_state(TransferId {
                value: "missing".into()
            })
            .await,
        Err(OkexClientError::ParameterClientIdNotFound)
    ));
    // Unmodelled endpoints produce a named fixture error through the real client.
    let result = client
        .withdraw_btc_onchain(
            ClientTransferId::new(),
            dec!(0.01),
            dec!(0.0002),
            "address".into(),
        )
        .await;
    assert!(
        matches!(result, Err(OkexClientError::UnexpectedResponse { msg, .. }) if msg.contains("Unmodelled endpoint: POST /api/v5/asset/withdrawal"))
    );
    exchange.assert_all_replies_consumed().await;
    Ok(())
}

#[tokio::test]
async fn closes_flat_positions_and_propagates_other_errors() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let client = exchange.client().await?;
    client.close_positions(ClientOrderId::new()).await?;
    for (code, msg) in [
        ("51023", "no position"),
        ("other", "Position does not exist"),
        ("other", "Position doesn't exist"),
    ] {
        exchange
            .reply_once("POST", CLOSE, StatusCode::OK, error(code, msg))
            .await;
        client.close_positions(ClientOrderId::new()).await?;
    }
    exchange
        .reply_once(
            "POST",
            CLOSE,
            StatusCode::BAD_REQUEST,
            error("other", "actual failure"),
        )
        .await;
    assert!(
        matches!(client.close_positions(ClientOrderId::new()).await, Err(OkexClientError::UnexpectedResponse { msg, .. }) if msg == "actual failure")
    );
    exchange.assert_all_replies_consumed().await;
    Ok(())
}

#[tokio::test]
async fn distinguishes_flat_and_malformed_position_data() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let client = exchange.client().await?;
    assert_eq!(
        client.get_position_in_signed_usd_cents().await?.usd_cents,
        dec!(0)
    );
    // Populate every field required by OKX's position response contract.
    let mut position = json!({});
    for field in [
        "adl",
        "availPos",
        "avgPx",
        "cTime",
        "ccy",
        "deltaBS",
        "deltaPA",
        "gammaBS",
        "gammaPA",
        "imr",
        "instId",
        "instType",
        "interest",
        "usdPx",
        "last",
        "lever",
        "liab",
        "liabCcy",
        "liqPx",
        "markPx",
        "margin",
        "mgnMode",
        "mgnRatio",
        "mmr",
        "notionalUsd",
        "optVal",
        "pos",
        "posCcy",
        "posId",
        "posSide",
        "thetaBS",
        "thetaPA",
        "tradeId",
        "uTime",
        "upl",
        "uplRatio",
        "vegaBS",
        "vegaPA",
    ] {
        position[field] = json!("");
    }
    for (pos, notional, last, expected) in [
        ("0", "", "", Some(dec!(0))),
        ("1", "100", "50000", Some(dec!(10000))),
        ("-1", "100", "50000", Some(dec!(-10000))),
        ("1", "bad", "50000", None),
        ("1", "100", "bad", None),
        ("bad", "100", "50000", None),
    ] {
        position["pos"] = json!(pos);
        position["notionalUsd"] = json!(notional);
        position["last"] = json!(last);
        exchange
            .reply_once(
                "GET",
                POSITIONS,
                StatusCode::OK,
                json!({"code":"0","msg":"","data":[position]}),
            )
            .await;
        match expected {
            Some(value) => assert_eq!(
                client.get_position_in_signed_usd_cents().await?.usd_cents,
                value
            ),
            None => assert!(matches!(
                client.get_position_in_signed_usd_cents().await,
                Err(OkexClientError::NonParsablePositionData)
            )),
        }
    }
    exchange.assert_all_replies_consumed().await;
    Ok(())
}

#[tokio::test]
async fn funding_history_and_withdrawal_contracts() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let client = exchange.client().await?;
    let deposit = json!({"actualDepBlkConfirm":"1","amt":"0.01","ccy":"BTC","chain":"BTC-Bitcoin","depId":"1","from":"sender","state":"2","to":"address","ts":"0","txId":"deposit-tx"});
    for (state, expected) in [
        ("0", "pending"),
        ("1", "pending"),
        ("2", "success"),
        ("8", "pending"),
        ("12", "pending"),
        ("13", "success"),
        ("bad", "failed"),
    ] {
        let mut data = deposit.clone();
        data["state"] = json!(state);
        exchange
            .reply_once(
                "GET",
                DEPOSITS,
                StatusCode::OK,
                json!({"code":"0","msg":"","data":[data]}),
            )
            .await;
        let result = client.fetch_deposit("address".into(), dec!(0.01)).await?;
        assert_eq!(result.state, expected);
        assert_eq!(result.transaction_id, "deposit-tx");
    }
    let id = ClientTransferId::new();
    let id_string = String::from(id.clone());
    let path = format!("/api/v5/asset/withdrawal-history?ccy=BTC&clientId={id_string}");
    assert!(matches!(
        client.fetch_withdrawal_by_client_id(id.clone()).await,
        Err(OkexClientError::ParameterClientIdNotFound)
    ));
    exchange.reply_once("POST", "/api/v5/asset/withdrawal", StatusCode::OK, json!({"code":"0","msg":"","data":[{"amt":"0.01","wdId":"1","ccy":"BTC","clientId":id_string,"chain":"BTC-Bitcoin"}]})).await;
    assert_eq!(
        client
            .withdraw_btc_onchain(id.clone(), dec!(0.01), dec!(0.0002), "address".into())
            .await?
            .value,
        "1"
    );
    let bodies = exchange
        .request_bodies("POST", "/api/v5/asset/withdrawal")
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&bodies[0])?,
        json!({"ccy":"BTC","amt":"0.01","dest":"4","fee":"0.0002","chain":"BTC-Bitcoin","toAddr":"address","clientId":id_string})
    );
    for (state, expected) in [
        ("-3", "pending"),
        ("-2", "failed"),
        ("-1", "failed"),
        ("0", "pending"),
        ("1", "pending"),
        ("2", "success"),
        ("7", "pending"),
        ("10", "pending"),
        ("4", "pending"),
        ("5", "pending"),
        ("6", "pending"),
        ("8", "pending"),
        ("9", "pending"),
        ("12", "pending"),
        ("bad", "failed"),
    ] {
        exchange.reply_once("GET", &path, StatusCode::OK, json!({"code":"0","msg":"","data":[{"ccy":"BTC","chain":"BTC-Bitcoin","amt":"0.01","ts":"0","from":"sender","to":"address","txId":"withdrawal-tx","state":state,"wdId":"1","clientId":id_string}]})).await;
        let result = client.fetch_withdrawal_by_client_id(id.clone()).await?;
        assert_eq!(result.state, expected);
        assert_eq!(result.transaction_id, "withdrawal-tx");
        assert_eq!(result.client_id, id_string);
    }
    exchange.assert_all_replies_consumed().await;
    Ok(())
}

#[tokio::test]
async fn constructor_rejects_misconfigured_accounts() {
    let exchange = Exchange::start().await;
    for (mode, level, message) in [
        ("long_short_mode", "2", "net_mode"),
        ("net_mode", "1", "acct_lv: 2"),
    ] {
        exchange.reply_once("GET", "/api/v5/account/config", StatusCode::OK, json!({"code":"0","msg":"","data":[{
            "acctLv":level,"autoLoan":false,"ctIsoMode":"automatic","greeksType":"PA","level":"Lv1","levelTmp":"","mgnIsoMode":"automatic","posMode":mode,"uid":"test"
        }]})).await;
        assert!(
            matches!(exchange.client().await, Err(OkexClientError::MisconfiguredAccount(msg)) if msg.contains(message))
        );
    }
    exchange.assert_all_replies_consumed().await;
}

#[tokio::test]
#[should_panic(expected = "GET /mistyped?ccy=BTC (1 replies)")]
async fn unused_scripted_reply_is_reported() {
    let exchange = Exchange::start().await;
    exchange
        .reply_once("GET", "/mistyped?ccy=BTC", StatusCode::OK, json!({}))
        .await;
    exchange.assert_all_replies_consumed().await;
}

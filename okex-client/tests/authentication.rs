use chrono::{SecondsFormat, Utc};
use data_encoding::BASE64;
use okex_client::test_support::Exchange;
use reqwest::{header::HeaderMap, Client, Method, StatusCode};
use ring::hmac;
use std::time::Duration;

fn signed_headers(method: &str, path: &str, body: &str, timestamp: &str) -> HeaderMap {
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, b"test-secret"),
        format!("{timestamp}{method}{path}{body}").as_bytes(),
    );
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("x-simulated-trading", "1".to_string()),
        ("ok-access-key", "test-key".to_string()),
        ("ok-access-passphrase", "test-passphrase".to_string()),
        ("ok-access-timestamp", timestamp.to_string()),
        ("ok-access-sign", BASE64.encode(signature.as_ref())),
    ] {
        headers.insert(name, value.parse().unwrap());
    }
    headers
}

fn client() -> Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

#[tokio::test]
async fn rejects_missing_or_invalid_authentication() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let url = exchange.url();
    let client = client();
    let path = "/api/v5/account/config";
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let valid = signed_headers("GET", path, "", &timestamp);
    for name in [
        "x-simulated-trading",
        "ok-access-key",
        "ok-access-passphrase",
        "ok-access-timestamp",
        "ok-access-sign",
    ] {
        let mut headers = valid.clone();
        headers.remove(name);
        assert_eq!(
            client
                .get(format!("{url}{path}"))
                .headers(headers)
                .send()
                .await?
                .status(),
            StatusCode::UNAUTHORIZED,
            "missing {name}"
        );
        let mut headers = valid.clone();
        headers.insert(name, "invalid".parse()?);
        assert_eq!(
            client
                .get(format!("{url}{path}"))
                .headers(headers)
                .send()
                .await?
                .status(),
            StatusCode::UNAUTHORIZED,
            "invalid {name}"
        );
    }
    for seconds in [-60, 60] {
        let timestamp = (Utc::now() + chrono::Duration::seconds(seconds))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        assert_eq!(
            client
                .get(format!("{url}{path}"))
                .headers(signed_headers("GET", path, "", &timestamp))
                .send()
                .await?
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    Ok(())
}

#[tokio::test]
async fn rejects_signatures_for_different_methods_paths_queries_and_bodies() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let url = exchange.url();
    let client = client();
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    for (signed_method, signed_path, signed_body, method, path, body) in [
        (
            "GET",
            "/api/v5/trade/order",
            "{}",
            Method::POST,
            "/api/v5/trade/order",
            "{}",
        ),
        (
            "GET",
            "/api/v5/account/config",
            "",
            Method::GET,
            "/api/v5/account/positions?instId=BTC-USD-SWAP",
            "",
        ),
        (
            "GET",
            "/api/v5/account/positions?instId=ETH-USD-SWAP",
            "",
            Method::GET,
            "/api/v5/account/positions?instId=BTC-USD-SWAP",
            "",
        ),
        (
            "POST",
            "/api/v5/trade/order",
            "{}",
            Method::POST,
            "/api/v5/trade/order",
            "{ \"sz\": \"2\" }",
        ),
    ] {
        let response = client
            .request(method, format!("{url}{path}"))
            .headers(signed_headers(
                signed_method,
                signed_path,
                signed_body,
                &timestamp,
            ))
            .body(body)
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[tokio::test]
async fn rejects_incorrect_endpoint_queries_and_oversized_bodies() -> anyhow::Result<()> {
    let exchange = Exchange::start().await;
    let url = exchange.url();
    let client = client();
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    for path in [
        "/api/v5/account/config?extra=1",
        "/api/v5/account/leverage-info?instId=BTC-USD-SWAP",
        "/api/v5/account/leverage-info?instId=BTC-USD-SWAP&mgnMode=isolated",
        "/api/v5/account/positions?instId=ETH-USD-SWAP",
        "/api/v5/market/ticker?instId=ETH-USD-SWAP",
        "/api/v5/asset/balances?ccy=USD",
        "/api/v5/asset/balances?ccy=%FF",
        "/api/v5/account/balance?ccy=USD",
        "/api/v5/asset/currencies?ccy=USD",
        "/api/v5/trade/order?instId=BTC-USD-SWAP",
        "/api/v5/trade/order?instId=BTC-USD-SWAP&clOrdId=",
        "/api/v5/trade/order?instId=ETH-USD-SWAP&clOrdId=test",
        "/api/v5/asset/transfer-state?ccy=BTC",
        "/api/v5/asset/transfer-state?ccy=BTC&clientId=",
        "/api/v5/asset/transfer-state?ccy=BTC&transId=",
        "/api/v5/asset/transfer-state?ccy=BTC&transId=1&clientId=test",
        "/api/v5/asset/transfer-state?ccy=USD&clientId=test",
    ] {
        assert_eq!(
            client
                .get(format!("{url}{path}"))
                .headers(signed_headers("GET", path, "", &timestamp))
                .send()
                .await?
                .status(),
            StatusCode::BAD_REQUEST,
            "{path}"
        );
    }
    let path = "/api/v5/trade/order";
    let body = " ".repeat(64 * 1024 + 1);
    assert_eq!(
        client
            .post(format!("{url}{path}"))
            .headers(signed_headers("POST", path, &body, &timestamp))
            .body(body)
            .send()
            .await?
            .status(),
        StatusCode::BAD_REQUEST
    );
    Ok(())
}

#[tokio::test]
async fn fixture_rejections_include_diagnostics_and_preserve_state() -> anyhow::Result<()> {
    use okex_client::{BtcUsdSwapContracts, ClientOrderId, OkexOrderSide};
    use rust_decimal_macros::dec;
    use serde_json::{json, Value};
    let exchange = Exchange::start().await;
    let okex = exchange.client().await?;
    let http = client();
    let order = json!({"instId":"BTC-USD-SWAP","tdMode":"cross","posSide":"net","ordType":"market","sz":"1","side":"sell","clOrdId":"order"});
    let transfer = json!({"amt":"0.01","ccy":"BTC","from":"6","to":"18","clientId":"transfer"});
    let mut cases = Vec::new();
    for (field, value, message) in [
        ("instId", "ETH-USD-SWAP", "instId"),
        ("tdMode", "isolated", "tdMode"),
        ("posSide", "long", "posSide"),
        ("ordType", "limit", "ordType"),
        ("sz", "bad", "Invalid order size"),
        ("sz", "0", "positive"),
        ("side", "bad", "order side"),
        ("clOrdId", "", "clOrdId"),
    ] {
        let mut body = order.clone();
        body[field] = json!(value);
        cases.push(("/api/v5/trade/order", body, message));
    }
    for (field, value, message) in [
        ("amt", "bad", "Invalid transfer amount"),
        ("amt", "0", "positive"),
        ("amt", "2", "Insufficient funding"),
        ("ccy", "USD", "ccy"),
        ("clientId", "", "clientId"),
        ("from", "bad", "transfer accounts"),
    ] {
        let mut body = transfer.clone();
        body[field] = json!(value);
        cases.push(("/api/v5/asset/transfer", body, message));
    }
    cases.push((
        "/api/v5/asset/transfer",
        json!({"amt":"1","ccy":"BTC","from":"18","to":"6","clientId":"insufficient"}),
        "Insufficient trading",
    ));
    cases.push((
        "/api/v5/trade/close-position",
        json!({"instId":"wrong","mgnMode":"cross"}),
        "instId",
    ));
    cases.push((
        "/api/v5/trade/close-position",
        json!({"instId":"BTC-USD-SWAP","mgnMode":"wrong"}),
        "mgnMode",
    ));
    for (path, body, expected) in cases {
        let body = body.to_string();
        let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let response = http
            .post(format!("{}{path}", exchange.url()))
            .headers(signed_headers("POST", path, &body, &timestamp))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let data: Value = response.json().await?;
        assert!(data["msg"].as_str().unwrap().contains(expected), "{data}");
    }
    assert_eq!(
        okex.get_position_in_signed_usd_cents().await?.usd_cents,
        dec!(0)
    );
    assert_eq!(
        okex.funding_account_balance().await?.total_amt_in_btc,
        dec!(1)
    );
    assert_eq!(
        okex.trading_account_balance().await?.total_amt_in_btc,
        dec!(0.01)
    );
    let id = ClientOrderId::new();
    okex.place_order(
        id.clone(),
        OkexOrderSide::Sell,
        &BtcUsdSwapContracts::from(1),
    )
    .await?;
    let error = okex
        .place_order(id, OkexOrderSide::Sell, &BtcUsdSwapContracts::from(1))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Duplicate client order ID"));
    assert_eq!(
        okex.get_position_in_signed_usd_cents().await?.usd_cents,
        dec!(-10000)
    );
    let id = okex_client::ClientTransferId::new();
    okex.transfer_funding_to_trading(id.clone(), dec!(0.01))
        .await?;
    assert!(okex
        .transfer_funding_to_trading(id, dec!(0.01))
        .await
        .unwrap_err()
        .to_string()
        .contains("Duplicate client transfer ID"));
    assert_eq!(
        okex.trading_account_balance().await?.total_amt_in_btc,
        dec!(0.02)
    );
    Ok(())
}

#[tokio::test]
async fn unknown_routes_with_queries_support_diagnostics_and_scripted_replies() -> anyhow::Result<()>
{
    use okex_client::test_support::StatusCode as FixtureStatus;
    use serde_json::{json, Value};
    let exchange = Exchange::start().await;
    let http = client();
    let path = "/api/v5/asset/deposit-address?ccy=BTC";
    for scripted in [false, true] {
        if scripted {
            exchange
                .reply_once(
                    "GET",
                    path,
                    FixtureStatus::OK,
                    json!({"code":"0","msg":"","data":[{"addr":"test-address"}]}),
                )
                .await;
        }
        let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let response = http
            .get(format!("{}{path}", exchange.url()))
            .headers(signed_headers("GET", path, "", &timestamp))
            .send()
            .await?;
        assert_eq!(
            response.status(),
            if scripted {
                StatusCode::OK
            } else {
                StatusCode::NOT_IMPLEMENTED
            }
        );
        let body: Value = response.json().await?;
        if scripted {
            assert_eq!(body["data"][0]["addr"], "test-address");
        } else {
            assert_eq!(body["msg"], format!("Unmodelled endpoint: GET {path}"));
        }
    }
    exchange.assert_all_replies_consumed().await;
    Ok(())
}

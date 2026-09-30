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
    let url = exchange.config().test_api.unwrap();
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
    let url = exchange.config().test_api.unwrap();
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
    let url = exchange.config().test_api.unwrap();
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

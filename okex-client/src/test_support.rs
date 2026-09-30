use crate::OkexClientConfig;
use axum::{
    body::{to_bytes, Body},
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use data_encoding::BASE64;
use ring::hmac;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

/// A separate exchange per test. Requests still use the real HTTP client and codecs.
pub struct Exchange {
    url: String,
    server: JoinHandle<()>,
}

struct Account {
    contracts: i64,
    orders: HashMap<String, Value>,
    trading: Decimal,
    funding: Decimal,
    transfers: HashMap<String, Value>,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            contracts: 0,
            orders: HashMap::new(),
            trading: Decimal::new(1, 2),
            funding: Decimal::ONE,
            transfers: HashMap::new(),
        }
    }
}

impl Account {
    fn record_order(&mut self, client_id: &str, size: i64) -> String {
        assert!(
            !self.orders.contains_key(client_id),
            "Duplicate client order ID"
        );
        let order_id = (self.orders.len() + 1).to_string();
        self.orders.insert(
            client_id.into(),
            json!({
                "clOrdId": client_id, "ordId": order_id, "avgPx": "50000", "fee": "0",
                "sz": size.to_string(), "state": "filled"
            }),
        );
        order_id
    }
}

type ExchangeState = Arc<Mutex<Account>>;

impl Exchange {
    pub async fn start() -> Self {
        let app = Router::new()
            .route("/api/v5/account/config", get(account_config))
            .route("/api/v5/account/leverage-info", get(leverage))
            .route("/api/v5/account/positions", get(positions))
            .route("/api/v5/market/ticker", get(ticker))
            .route("/api/v5/asset/balances", get(funding_balance))
            .route("/api/v5/account/balance", get(trading_balance))
            .route("/api/v5/asset/currencies", get(fees))
            .route("/api/v5/asset/transfer", post(transfer))
            .route("/api/v5/asset/transfer-state", get(transfer_state))
            .route("/api/v5/trade/order", post(order).get(order_details))
            .route("/api/v5/trade/close-position", post(close))
            .route_layer(middleware::from_fn(authenticate))
            .with_state(Arc::new(Mutex::new(Account::default())));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, server }
    }

    pub fn config(&self) -> OkexClientConfig {
        OkexClientConfig {
            api_key: "test-key".into(),
            secret_key: "test-secret".into(),
            passphrase: "test-passphrase".into(),
            simulated: true,
            test_api: Some(self.url.clone()),
        }
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn response(data: Value) -> Json<Value> {
    Json(json!({"code": "0", "msg": "", "data": data}))
}

async fn authenticate(request: Request, next: Next) -> Result<Response, StatusCode> {
    let (parts, body) = request.into_parts();
    let header = |name| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    for (name, expected) in [
        ("x-simulated-trading", "1"),
        ("ok-access-key", "test-key"),
        ("ok-access-passphrase", "test-passphrase"),
    ] {
        if header(name) != Some(expected) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    let timestamp = header("ok-access-timestamp").ok_or(StatusCode::UNAUTHORIZED)?;
    let parsed =
        chrono::DateTime::parse_from_rfc3339(timestamp).map_err(|_| StatusCode::UNAUTHORIZED)?;
    if (chrono::Utc::now() - parsed.with_timezone(&chrono::Utc)).abs()
        > chrono::Duration::seconds(30)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let signature = BASE64
        .decode(
            header("ok-access-sign")
                .ok_or(StatusCode::UNAUTHORIZED)?
                .as_bytes(),
        )
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut message = format!("{timestamp}{}{}", parts.method, parts.uri).into_bytes();
    message.extend_from_slice(&body);
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, b"test-secret"),
        &message,
        &signature,
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let Query(query) = Query::<HashMap<String, String>>::try_from_uri(&parts.uri)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let expected: &[(&str, &str)] = match parts.uri.path() {
        "/api/v5/account/leverage-info" => &[("instId", "BTC-USD-SWAP"), ("mgnMode", "cross")],
        "/api/v5/account/positions" | "/api/v5/market/ticker" => &[("instId", "BTC-USD-SWAP")],
        "/api/v5/asset/balances" | "/api/v5/account/balance" | "/api/v5/asset/currencies" => {
            &[("ccy", "BTC")]
        }
        "/api/v5/trade/order" if parts.method == axum::http::Method::GET => &[
            ("instId", "BTC-USD-SWAP"),
            (
                "clOrdId",
                query
                    .get("clOrdId")
                    .filter(|id| !id.is_empty())
                    .ok_or(StatusCode::BAD_REQUEST)?,
            ),
        ],
        "/api/v5/asset/transfer-state" => &[
            ("ccy", "BTC"),
            (
                "clientId",
                query
                    .get("clientId")
                    .filter(|id| !id.is_empty())
                    .ok_or(StatusCode::BAD_REQUEST)?,
            ),
        ],
        _ => &[],
    };
    if query.len() != expected.len()
        || expected
            .iter()
            .any(|(key, value)| query.get(*key).map(String::as_str) != Some(*value))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(next.run(Request::from_parts(parts, Body::from(body))).await)
}

async fn account_config() -> Json<Value> {
    response(json!([{
        "acctLv": "2", "autoLoan": false, "ctIsoMode": "automatic",
        "greeksType": "PA", "level": "Lv1", "levelTmp": "",
        "mgnIsoMode": "automatic", "posMode": "net_mode", "uid": "test"
    }]))
}

async fn leverage() -> Json<Value> {
    response(json!([{
        "instId": "BTC-USD-SWAP", "mgnMode": "cross", "posSide": "net", "lever": "4"
    }]))
}

async fn ticker() -> Json<Value> {
    response(json!([{
        "instType": "SWAP", "instId": "BTC-USD-SWAP", "last": "50000",
        "lastSz": "1", "askPx": "50000", "askSz": "1", "bidPx": "50000", "bidSz": "1"
    }]))
}

async fn funding_balance(State(account): State<ExchangeState>) -> Json<Value> {
    let balance = account.lock().await.funding.to_string();
    response(json!([{"availBal": balance, "bal": balance, "ccy": "BTC", "frozenBal": "0"}]))
}

async fn trading_balance(State(account): State<ExchangeState>) -> Json<Value> {
    let collateral = account.lock().await.trading;
    let mut details = serde_json::Map::new();
    for field in [
        "availBal",
        "availEq",
        "cashBal",
        "ccy",
        "crossLiab",
        "disEq",
        "eq",
        "eqUsd",
        "frozenBal",
        "interest",
        "isoEq",
        "isoLiab",
        "isoUpl",
        "liab",
        "maxLoan",
        "mgnRatio",
        "notionalLever",
        "ordFrozen",
        "twap",
        "uTime",
        "upl",
        "uplLiab",
        "stgyEq",
        "spotInUseAmt",
    ] {
        details.insert(field.into(), json!("0"));
    }
    details.insert("ccy".into(), json!("BTC"));
    details.insert("availEq".into(), json!(collateral.to_string()));
    details.insert("eq".into(), json!(collateral.to_string()));
    response(json!([{
        "adjEq": "", "details": [details], "imr": "", "isoEq": "", "mgnRatio": "",
        "mmr": "", "notionalUsd": "", "ordFroz": "", "totalEq": "", "uTime": ""
    }]))
}

async fn transfer(State(account): State<ExchangeState>, Json(body): Json<Value>) -> Json<Value> {
    let amount: Decimal = body["amt"].as_str().unwrap().parse().unwrap();
    assert!(amount > Decimal::ZERO);
    assert_eq!(body["ccy"], "BTC");
    let mut account = account.lock().await;
    match (body["from"].as_str().unwrap(), body["to"].as_str().unwrap()) {
        ("6", "18") => {
            assert!(account.funding >= amount);
            account.funding -= amount;
            account.trading += amount;
        }
        ("18", "6") => {
            assert!(account.trading >= amount);
            account.trading -= amount;
            account.funding += amount;
        }
        other => panic!("Unexpected transfer: {other:?}"),
    }
    let id = body["clientId"].as_str().unwrap();
    let mut data = body.clone();
    data["transId"] = json!((account.transfers.len() + 1).to_string());
    data["state"] = json!("success");
    data["subAcct"] = json!("");
    account.transfers.insert(id.into(), data.clone());
    response(json!([data]))
}

async fn transfer_state(
    State(account): State<ExchangeState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    let account = account.lock().await;
    response(json!([account.transfers[&query["clientId"]]]))
}

async fn fees() -> Json<Value> {
    response(json!([{
        "ccy": "BTC", "chain": "BTC-Bitcoin", "minFee": "0.0002", "maxFee": "0.0004",
        "minWd": "0.001", "maxWd": "500"
    }]))
}

async fn positions(State(account): State<ExchangeState>) -> Json<Value> {
    let contracts = account.lock().await.contracts;
    if contracts == 0 {
        return response(json!([]));
    }
    let mut position = serde_json::Map::new();
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
        position.insert(field.into(), json!(""));
    }
    position.insert("pos".into(), json!(contracts.to_string()));
    position.insert(
        "notionalUsd".into(),
        json!((contracts.abs() * 100).to_string()),
    );
    position.insert("last".into(), json!("50000"));
    response(json!([position]))
}

async fn order(State(account): State<ExchangeState>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(body["instId"], "BTC-USD-SWAP");
    assert_eq!(body["tdMode"], "cross");
    assert_eq!(body["posSide"], "net");
    assert_eq!(body["ordType"], "market");
    let size = body["sz"].as_str().unwrap().parse::<i64>().unwrap();
    assert!(size > 0);
    let direction = match body["side"].as_str().unwrap() {
        "buy" => 1,
        "sell" => -1,
        other => panic!("Unexpected order side: {other}"),
    };
    let id = body["clOrdId"].as_str().unwrap();
    let mut account = account.lock().await;
    account.contracts += direction * size;
    let order_id = account.record_order(id, size);
    response(json!([{"clOrdId": id, "ordId": order_id, "tag": "", "sCode": "0", "sMsg": ""}]))
}

async fn order_details(
    State(account): State<ExchangeState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    let account = account.lock().await;
    response(json!([account.orders[&query["clOrdId"]]]))
}

async fn close(State(account): State<ExchangeState>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(body["instId"], "BTC-USD-SWAP");
    assert_eq!(body["mgnMode"], "cross");
    let mut account = account.lock().await;
    let size = account.contracts.abs();
    account.record_order(body["clOrdId"].as_str().unwrap(), size);
    account.contracts = 0;
    response(json!([{"instId": "BTC-USD-SWAP", "posSide": "net"}]))
}

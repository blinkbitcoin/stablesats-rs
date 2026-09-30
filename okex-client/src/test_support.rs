use crate::{OkexClient, OkexClientConfig, OkexClientError};
pub use axum::http::StatusCode;
use axum::{
    body::{to_bytes, Body},
    extract::{Query, Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use data_encoding::BASE64;
use ring::hmac;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

/// A separate exchange per test. Requests still use the real HTTP client and codecs.
pub struct Exchange {
    url: String,
    server: JoinHandle<()>,
    account: ExchangeState,
}

struct Account {
    contracts: i64,
    orders: HashMap<String, Value>,
    trading: Decimal,
    funding: Decimal,
    transfers: HashMap<String, Value>,
    replies: HashMap<String, VecDeque<(StatusCode, String)>>,
    requests: HashMap<String, Vec<String>>,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            contracts: 0,
            orders: HashMap::new(),
            trading: Decimal::new(1, 2),
            funding: Decimal::ONE,
            transfers: HashMap::new(),
            replies: HashMap::new(),
            requests: HashMap::new(),
        }
    }
}

impl Account {
    fn record_order(&mut self, client_id: &str, size: i64) -> Result<String, FixtureError> {
        ensure(
            !self.orders.contains_key(client_id),
            "Duplicate client order ID",
        )?;
        let order_id = (self.orders.len() + 1).to_string();
        self.orders.insert(
            client_id.into(),
            json!({
                "clOrdId": client_id, "ordId": order_id, "avgPx": "50000", "fee": "0",
                "sz": size.to_string(), "state": "filled"
            }),
        );
        Ok(order_id)
    }
}

type ExchangeState = Arc<Mutex<Account>>;

impl Exchange {
    pub async fn start() -> Self {
        let account = Arc::new(Mutex::new(Account::default()));
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
            .route("/api/v5/asset/deposit-history", get(empty_history))
            .route("/api/v5/asset/withdrawal-history", get(empty_history))
            .fallback(unmodelled_endpoint)
            .layer(middleware::from_fn_with_state(
                account.clone(),
                authenticate,
            ))
            .with_state(account.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            server,
            account,
        }
    }

    /// Queue an exact response after authentication, for production-client error-path tests.
    pub async fn reply_once(&self, method: &str, path: &str, status: StatusCode, body: Value) {
        self.reply_raw_once(method, path, status, body.to_string())
            .await;
    }

    pub async fn reply_raw_once(&self, method: &str, path: &str, status: StatusCode, body: String) {
        self.account
            .lock()
            .await
            .replies
            .entry(format!("{method} {path}"))
            .or_default()
            .push_back((status, body));
    }

    pub async fn request_bodies(&self, method: &str, path: &str) -> Vec<String> {
        self.account
            .lock()
            .await
            .requests
            .get(&format!("{method} {path}"))
            .cloned()
            .unwrap_or_default()
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub async fn client(&self) -> Result<OkexClient, OkexClientError> {
        OkexClient::with_test_endpoint(self.config(), self.url.clone()).await
    }

    pub fn config(&self) -> OkexClientConfig {
        OkexClientConfig {
            api_key: "test-key".into(),
            secret_key: "test-secret".into(),
            passphrase: "test-passphrase".into(),
            simulated: true,
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

#[derive(Debug)]
struct FixtureError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl FixtureError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

impl IntoResponse for FixtureError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code": self.code, "msg": self.message, "data": null})),
        )
            .into_response()
    }
}

impl From<StatusCode> for FixtureError {
    fn from(status: StatusCode) -> Self {
        Self::new(
            status,
            "fixture",
            format!("Fixture rejected request: {status}"),
        )
    }
}

type FixtureResult = Result<Json<Value>, FixtureError>;

fn ensure(condition: bool, message: impl Into<String>) -> Result<(), FixtureError> {
    if condition {
        Ok(())
    } else {
        Err(FixtureError::new(
            StatusCode::BAD_REQUEST,
            "fixture",
            message,
        ))
    }
}

fn field<'a>(body: &'a Value, name: &str) -> Result<&'a str, FixtureError> {
    body.get(name)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            FixtureError::new(
                StatusCode::BAD_REQUEST,
                "fixture",
                format!("Missing or invalid field {name}"),
            )
        })
}

fn expect_field(body: &Value, name: &str, expected: &str) -> Result<(), FixtureError> {
    ensure(
        field(body, name)? == expected,
        format!("Expected {name}={expected}"),
    )
}

async fn unmodelled_endpoint(request: Request) -> FixtureError {
    FixtureError::new(
        StatusCode::NOT_IMPLEMENTED,
        "fixture",
        format!(
            "Unmodelled endpoint: {} {}",
            request.method(),
            request.uri()
        ),
    )
}

async fn empty_history() -> Json<Value> {
    response(json!([]))
}

async fn authenticate(
    State(account): State<ExchangeState>,
    request: Request,
    next: Next,
) -> Result<Response, FixtureError> {
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
            return Err(StatusCode::UNAUTHORIZED.into());
        }
    }
    let timestamp = header("ok-access-timestamp").ok_or(StatusCode::UNAUTHORIZED)?;
    let parsed =
        chrono::DateTime::parse_from_rfc3339(timestamp).map_err(|_| StatusCode::UNAUTHORIZED)?;
    if (chrono::Utc::now() - parsed.with_timezone(&chrono::Utc)).abs()
        > chrono::Duration::seconds(30)
    {
        return Err(FixtureError::new(
            StatusCode::UNAUTHORIZED,
            "50102",
            "Timestamp expired",
        ));
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
        "/api/v5/asset/transfer-state" | "/api/v5/asset/withdrawal-history" => &[
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
        return Err(StatusCode::BAD_REQUEST.into());
    }
    let key = format!("{} {}", parts.method, parts.uri);
    let mut account = account.lock().await;
    account
        .requests
        .entry(key.clone())
        .or_default()
        .push(String::from_utf8_lossy(&body).into_owned());
    if let Some((status, body)) = account.replies.get_mut(&key).and_then(VecDeque::pop_front) {
        return Ok((
            status,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response());
    }
    drop(account);
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

async fn transfer(State(account): State<ExchangeState>, Json(body): Json<Value>) -> FixtureResult {
    let amount: Decimal = field(&body, "amt")?.parse().map_err(|_| {
        FixtureError::new(
            StatusCode::BAD_REQUEST,
            "fixture",
            "Invalid transfer amount",
        )
    })?;
    ensure(amount > Decimal::ZERO, "Transfer amount must be positive")?;
    expect_field(&body, "ccy", "BTC")?;
    let id = field(&body, "clientId")?;
    let mut account = account.lock().await;
    ensure(
        !account.transfers.contains_key(id),
        "Duplicate client transfer ID",
    )?;
    match (field(&body, "from")?, field(&body, "to")?) {
        ("6", "18") => {
            ensure(account.funding >= amount, "Insufficient funding balance")?;
            account.funding -= amount;
            account.trading += amount;
        }
        ("18", "6") => {
            ensure(account.trading >= amount, "Insufficient trading balance")?;
            account.trading -= amount;
            account.funding += amount;
        }
        _ => {
            return Err(FixtureError::new(
                StatusCode::BAD_REQUEST,
                "fixture",
                "Unexpected transfer accounts",
            ))
        }
    }
    let mut data = body.clone();
    data["transId"] = json!((account.transfers.len() + 1).to_string());
    data["state"] = json!("success");
    data["subAcct"] = json!("");
    account.transfers.insert(id.into(), data.clone());
    Ok(response(json!([data])))
}

async fn transfer_state(
    State(account): State<ExchangeState>,
    Query(query): Query<HashMap<String, String>>,
) -> FixtureResult {
    let account = account.lock().await;
    let data = account.transfers.get(&query["clientId"]).ok_or_else(|| {
        FixtureError::new(
            StatusCode::BAD_REQUEST,
            "58129",
            "Unknown client transfer ID",
        )
    })?;
    Ok(response(json!([data])))
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

async fn order(State(account): State<ExchangeState>, Json(body): Json<Value>) -> FixtureResult {
    for (name, expected) in [
        ("instId", "BTC-USD-SWAP"),
        ("tdMode", "cross"),
        ("posSide", "net"),
        ("ordType", "market"),
    ] {
        expect_field(&body, name, expected)?;
    }
    let size = field(&body, "sz")?
        .parse::<i64>()
        .map_err(|_| FixtureError::new(StatusCode::BAD_REQUEST, "fixture", "Invalid order size"))?;
    ensure(size > 0, "Order size must be positive")?;
    let direction = match field(&body, "side")? {
        "buy" => 1,
        "sell" => -1,
        _ => {
            return Err(FixtureError::new(
                StatusCode::BAD_REQUEST,
                "fixture",
                "Unexpected order side",
            ))
        }
    };
    let id = field(&body, "clOrdId")?;
    let mut account = account.lock().await;
    let order_id = account.record_order(id, size)?;
    account.contracts += direction * size;
    Ok(response(
        json!([{"clOrdId": id, "ordId": order_id, "tag": "", "sCode": "0", "sMsg": ""}]),
    ))
}

async fn order_details(
    State(account): State<ExchangeState>,
    Query(query): Query<HashMap<String, String>>,
) -> FixtureResult {
    let account = account.lock().await;
    let data = account.orders.get(&query["clOrdId"]).ok_or_else(|| {
        FixtureError::new(StatusCode::BAD_REQUEST, "51603", "Unknown client order ID")
    })?;
    Ok(response(json!([data])))
}

async fn close(State(account): State<ExchangeState>, Json(body): Json<Value>) -> FixtureResult {
    expect_field(&body, "instId", "BTC-USD-SWAP")?;
    expect_field(&body, "mgnMode", "cross")?;
    let mut account = account.lock().await;
    let size = account.contracts.abs();
    if size == 0 {
        return Err(FixtureError::new(
            StatusCode::OK,
            "51023",
            "Position does not exist",
        ));
    }
    account.record_order(field(&body, "clOrdId")?, size)?;
    account.contracts = 0;
    Ok(response(
        json!([{"instId": "BTC-USD-SWAP", "posSide": "net"}]),
    ))
}

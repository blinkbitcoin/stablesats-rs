use galoy_client::GaloyClientConfig;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serial_test::{file_serial, serial};

use bria_client::*;
use ledger::*;
use okex_client::*;
use shared::pubsub::*;

use hedging::*;
use shared::test_utils::DatabaseTestFixture;

use okex_client::test_support as support;

// The hedging scenario does not call Galoy or Bria, but Bria connects over HTTP/2.
// Keep that connection local and fail if the scenario starts depending on either API.
struct ServiceConnections {
    url: String,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

impl ServiceConnections {
    async fn start() -> anyhow::Result<Self> {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
            observed.fetch_add(1, Ordering::SeqCst);
            async move {
                (
                    axum::http::StatusCode::NOT_IMPLEMENTED,
                    format!("Unexpected dependency request: {}", request.uri()),
                )
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Ok(Self { url, calls, server })
    }
}

impl Drop for ServiceConnections {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn service_connections_report_unexpected_requests() -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let services = ServiceConnections::start().await?;
    let mut socket =
        tokio::net::TcpStream::connect(services.url.trim_start_matches("http://")).await?;
    socket
        .write_all(b"GET /unexpected HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut response = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        socket.read_to_string(&mut response),
    )
    .await??;
    assert!(response.starts_with("HTTP/1.1 501"));
    assert!(response.contains("Unexpected dependency request: /unexpected"));
    assert_eq!(services.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    Ok(())
}

const POSITION_PHASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
const HEDGING_SETUP_BUDGET: std::time::Duration = std::time::Duration::from_secs(80);

#[tokio::test]
#[serial]
#[file_serial]
async fn hedging() -> anyhow::Result<()> {
    tokio::time::timeout(
        4 * POSITION_PHASE_TIMEOUT + HEDGING_SETUP_BUDGET,
        run_hedging(),
    )
    .await?
}

async fn wait_for_position(
    events: &mut tokio::sync::broadcast::Receiver<LedgerEvent>,
    label: &str,
    expected: Decimal,
) -> anyhow::Result<()> {
    let balances = futures::stream::unfold(events, |events| async {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let LedgerEventData::BalanceUpdated(balance) = event.data {
                        return Some((
                            Ok(balance.settled_cr_balance - balance.settled_dr_balance),
                            events,
                        ));
                    }
                }
                Err(error) => return Some((Err(error), events)),
            }
        }
    });
    wait_for_balance(balances, label, expected, POSITION_PHASE_TIMEOUT).await
}

async fn wait_for_balance(
    balances: impl futures::Stream<Item = Result<Decimal, tokio::sync::broadcast::error::RecvError>>,
    label: &str,
    expected: Decimal,
    timeout: std::time::Duration,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use tokio::sync::broadcast::error::RecvError;
    futures::pin_mut!(balances);
    let mut last = None;
    let result = tokio::time::timeout(timeout, async {
        while let Some(balance) = balances.next().await {
            match balance {
                Ok(balance) => {
                    last = Some(balance);
                    if balance == expected {
                        return Ok(());
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    eprintln!("{label}: skipped {n} lagged balance events")
                }
                Err(RecvError::Closed) => anyhow::bail!(
                    "{label}: balance channel closed; expected {expected}, last {last:?}"
                ),
            }
        }
        anyhow::bail!("{label}: balance stream ended; expected {expected}, last {last:?}")
    })
    .await;
    result.unwrap_or_else(|_| {
        Err(anyhow::anyhow!(
            "{label}: timed out; expected {expected}, last {last:?}"
        ))
    })
}

#[tokio::test]
async fn position_wait_reports_phase_and_last_balance_and_tolerates_lag() {
    use futures::{stream, StreamExt};
    use tokio::sync::broadcast::error::RecvError;
    let timeout = std::time::Duration::from_millis(200);
    wait_for_balance(
        stream::iter([Err(RecvError::Lagged(10)), Ok(dec!(-500))]),
        "re-hedge",
        dec!(-500),
        timeout,
    )
    .await
    .unwrap();
    let error = wait_for_balance(
        stream::iter([Ok(dec!(0))]).chain(stream::pending()),
        "initial hedge",
        dec!(-500),
        timeout,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("initial hedge")
            && error.contains("expected -500")
            && error.contains("last Some(0)")
            && error.contains("timed out"),
        "{error}"
    );
    for results in [vec![Err(RecvError::Closed)], vec![]] {
        let error = wait_for_balance(stream::iter(results), "close", dec!(0), timeout)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("close") && error.contains("expected 0") && error.contains("last None"),
            "{error}"
        );
    }
}

async fn run_hedging() -> anyhow::Result<()> {
    let services = ServiceConnections::start().await?;
    let exchange = support::Exchange::start().await;
    let okex = exchange.client().await?;
    let db_fixture = DatabaseTestFixture::new().await?;
    let pool = db_fixture.pool().clone();
    let ledger = Ledger::init(&pool).await?;
    let (_, health) = futures::channel::mpsc::unbounded();
    let (_, ticks) = memory::channel(chrono::Duration::seconds(1));
    let _app = HedgingApp::run_with_client(
        pool.clone(),
        health,
        HedgingAppConfig::default(),
        OkexConfig {
            poll_frequency: std::time::Duration::from_secs(1),
            ..Default::default()
        },
        GaloyClientConfig {
            api: format!("{}/graphql", services.url),
            api_key: "test-key".into(),
        },
        BriaClientConfig {
            url: services.url.clone(),
            profile_api_key: "test-key".into(),
            wallet_name: "test-wallet".into(),
            payout_queue_name: "test-queue".into(),
            onchain_address_external_id: "test-address".into(),
        },
        ticks,
        ledger.clone(),
        okex.clone(),
    )
    .await?;
    let mut events = ledger.usd_okex_position_balance_events().await?;

    ledger
        .user_buys_usd(
            pool.begin().await?,
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
    wait_for_position(&mut events, "initial hedge", dec!(-500)).await?;
    assert_eq!(
        okex.get_position_in_signed_usd_cents().await?.usd_cents,
        dec!(-50000)
    );

    // Observe the close and re-hedge through ordered ledger events. Polling the
    // exchange after a sleep can miss the zero position once hedging repairs it.
    okex.close_positions(ClientOrderId::new()).await?;
    wait_for_position(&mut events, "manual close", dec!(0)).await?;
    wait_for_position(&mut events, "re-hedge", dec!(-500)).await?;

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
    wait_for_position(&mut events, "liability removal", dec!(0)).await?;
    assert_eq!(
        okex.get_position_in_signed_usd_cents().await?.usd_cents,
        dec!(0)
    );
    assert_eq!(
        services.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "hedging scenario unexpectedly called Galoy or Bria"
    );
    Ok(())
}

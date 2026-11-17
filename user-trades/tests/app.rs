use rust_decimal_macros::dec;
use serial_test::{file_serial, serial};

use std::{env, time::Duration};

use galoy_client::GaloyClientConfig;

use ::user_trades::{user_trades::*, *};

const LEDGER_POSTING_TIMEOUT: Duration = Duration::from_secs(120);
const SEEDED_TX_ID_PREFIX: &str = "publishes-liability-";

// Stops the job runner and poll loop when the test body returns, including early returns.
// A poll job already in flight may still finish, but it only claims existing rows by id and
// cannot recreate seeded rows after they are deleted.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn galoy_client_configuration() -> GaloyClientConfig {
    let api = env::var("GALOY_GRAPHQL_URI").expect("GALOY_GRAPHQL_URI not set");
    let api_key = env::var("GALOY_API_KEY").expect("GALOY_API_KEY not set");

    GaloyClientConfig { api, api_key }
}

async fn ledger_tx_id_for(
    pool: &sqlx::PgPool,
    usd_tx_id: &str,
) -> anyhow::Result<Option<uuid::Uuid>> {
    Ok(sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT ledger_tx_id FROM user_trades WHERE external_ref->>'usd_tx_id' = $1",
    )
    .bind(usd_tx_id)
    .fetch_one(pool)
    .await?)
}

async fn ledger_tx_template_codes(
    pool: &sqlx::PgPool,
    ledger_tx_id: uuid::Uuid,
) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT t.code FROM sqlx_ledger_transactions tx
         JOIN sqlx_ledger_tx_templates t ON t.id = tx.tx_template_id
         WHERE tx.id = $1",
    )
    .bind(ledger_tx_id)
    .fetch_all(pool)
    .await?)
}

// UserTradesApp polls galoy and then posts every unaccounted user trade to the ledger.
// This test used to wait for an OKEX_ALLOCATION balance event, which only the hedging app and
// the ledger tests emit. It passed only when a concurrent test posted after the subscription
// and hung until the CI timeout otherwise. The quickstart dealer account has no trades, so seed
// a buy/sell pair (net zero liability) and wait until the app has posted both to the ledger.
// file_serial keeps it from interleaving with the ledger and hedging tests that assert balances.
#[tokio::test]
#[serial]
#[file_serial]
async fn publishes_liability() -> anyhow::Result<()> {
    let pg_host = std::env::var("PG_HOST").unwrap_or_else(|_| "localhost".into());
    let pg_port = std::env::var("PG_PORT").unwrap_or_else(|_| "5432".into());
    let pg_con = format!("postgres://user:password@{pg_host}:{pg_port}/pg");
    let pool = sqlx::PgPool::connect(&pg_con).await?;

    // Unposted rows make hedging's hack_user_trades_lag::lag_ok() refuse to hedge, and
    // DatabaseTestFixture resets every user_trades.ledger_tx_id to NULL. Remove seeded rows
    // (including leftovers from an aborted run) so later hedging tests on this DB still work.
    delete_seeded_trades(&pool).await?;
    let result = seed_trades_and_wait_for_ledger(&pool).await;
    delete_seeded_trades(&pool).await?;
    result
}

async fn delete_seeded_trades(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM user_trades WHERE external_ref->>'usd_tx_id' LIKE $1")
        .bind(format!("{SEEDED_TX_ID_PREFIX}%"))
        .execute(pool)
        .await?;
    Ok(())
}

async fn seed_trades_and_wait_for_ledger(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let pool = pool.clone();
    let ledger = ledger::Ledger::init(&pool).await?;

    let before = ledger.balances().usd_liability_balances().await?;

    let run_id = uuid::Uuid::new_v4();
    let buy_usd_tx_id = format!("{SEEDED_TX_ID_PREFIX}buy-usd-{run_id}");
    let sell_usd_tx_id = format!("{SEEDED_TX_ID_PREFIX}sell-usd-{run_id}");
    let now = chrono::Utc::now();
    // Read the env before seeding so a missing variable cannot leave unposted rows behind.
    let galoy_cfg = galoy_client_configuration();
    let mut tx = pool.begin().await?;
    UserTrades::new(pool.clone())
        .persist_all(
            &mut tx,
            vec![
                NewUserTrade {
                    buy_unit: UserTradeUnit::UsdCent,
                    buy_amount: dec!(500),
                    sell_unit: UserTradeUnit::Satoshi,
                    sell_amount: dec!(1000000),
                    external_ref: ExternalRef {
                        timestamp: now,
                        btc_tx_id: format!("{SEEDED_TX_ID_PREFIX}buy-btc-{run_id}"),
                        usd_tx_id: buy_usd_tx_id.clone(),
                    },
                },
                NewUserTrade {
                    buy_unit: UserTradeUnit::Satoshi,
                    buy_amount: dec!(1000000),
                    sell_unit: UserTradeUnit::UsdCent,
                    sell_amount: dec!(500),
                    external_ref: ExternalRef {
                        timestamp: now,
                        btc_tx_id: format!("{SEEDED_TX_ID_PREFIX}sell-btc-{run_id}"),
                        usd_tx_id: sell_usd_tx_id.clone(),
                    },
                },
            ],
        )
        .await?;
    tx.commit().await?;

    let mut app = AbortOnDrop(tokio::spawn(UserTradesApp::run(
        pool.clone(),
        UserTradesConfig {
            galoy_poll_frequency: Duration::from_secs(1),
        },
        galoy_cfg,
        ledger.clone(),
    )));

    let deadline = tokio::time::Instant::now() + LEDGER_POSTING_TIMEOUT;
    let (buy_ledger_tx_id, sell_ledger_tx_id) = loop {
        if let (Some(buy), Some(sell)) = (
            ledger_tx_id_for(&pool, &buy_usd_tx_id).await?,
            ledger_tx_id_for(&pool, &sell_usd_tx_id).await?,
        ) {
            break (buy, sell);
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "seeded user trades were not posted to the ledger within \
                 {LEDGER_POSTING_TIMEOUT:?}; the galoy poll must succeed before update_ledger runs"
            );
        }
        tokio::select! {
            res = &mut app.0 => {
                let reason = match res {
                    Ok(Ok(_)) => "returned without error".to_string(),
                    Ok(Err(e)) => format!("failed: {e}"),
                    Err(e) => format!("task failed: {e}"),
                };
                anyhow::bail!("UserTradesApp stopped before posting the seeded trades: {reason}");
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    };
    drop(app);

    // ensure! instead of assert! keeps failures on the Result path so the cleanup still runs.
    let buy_codes = ledger_tx_template_codes(&pool, buy_ledger_tx_id).await?;
    anyhow::ensure!(
        buy_codes == ["USER_BUYS_USD"],
        "buy trade posted with templates {buy_codes:?}"
    );
    let sell_codes = ledger_tx_template_codes(&pool, sell_ledger_tx_id).await?;
    anyhow::ensure!(
        sell_codes == ["USER_SELLS_USD"],
        "sell trade posted with templates {sell_codes:?}"
    );

    let after = ledger.balances().usd_liability_balances().await?;
    anyhow::ensure!(
        after.total_liability == before.total_liability
            && after.unallocated_usd == before.unallocated_usd,
        "net zero trades changed liabilities: before {before:?}, after {after:?}"
    );

    Ok(())
}

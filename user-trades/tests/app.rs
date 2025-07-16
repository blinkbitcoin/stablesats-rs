use rust_decimal_macros::dec;
use serial_test::serial;

use std::env;

use galoy_client::GaloyClientConfig;

use ::user_trades::*;

fn galoy_client_configuration() -> GaloyClientConfig {
    let api = env::var("GALOY_GRAPHQL_URI").expect("GALOY_GRAPHQL_URI not set");
    let phone_number = env::var("GALOY_PHONE_NUMBER").expect("GALOY_PHONE_NUMBER not set");
    let code = env::var("GALOY_PHONE_CODE").expect("GALOY_PHONE_CODE not set");

    let config = GaloyClientConfig {
        api,
        phone_number,
        auth_code: code,
    };

    config
}

#[tokio::test]
#[serial]
async fn publishes_liability() -> anyhow::Result<()> {
    let pg_host = std::env::var("PG_HOST").unwrap_or_else(|_| "localhost".into());
    let pg_port = std::env::var("PG_PORT").unwrap_or_else(|_| "5432".into());
    let pg_con = format!("postgres://user:password@{pg_host}:{pg_port}/pg");
    let pool = sqlx::PgPool::connect(&pg_con).await?;
    let ledger = ledger::Ledger::init(&pool).await?;
    let mut events = ledger.okex_usd_liability_balance_events().await?;
    let _ = tokio::spawn(UserTradesApp::run(
        pool,
        UserTradesConfig {
            galoy_poll_frequency: std::time::Duration::from_secs(1),
        },
        galoy_client_configuration(),
        ledger,
    ));

    let received = events.recv().await.unwrap();
    if let ledger::LedgerEventData::BalanceUpdated(data) = received.data {
        assert!(data.settled_dr_balance >= dec!(0));
    } else {
        assert!(false)
    }

    Ok(())
}

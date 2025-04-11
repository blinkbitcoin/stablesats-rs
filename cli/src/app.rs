use anyhow::Context;
use chrono::Duration;
use clap::{Parser, Subcommand};
use rust_decimal::Decimal;
use std::{collections::HashMap, path::PathBuf};
use url::Url;

use super::{config::*, price_client::*, quotes_client::*};
use shared::pubsub::memory;

#[derive(Parser)]
#[clap(version, long_about = None)]
struct Cli {
    /// Sets a custom config file
    #[clap(
        short,
        long,
        env = "STABLESATS_CONFIG",
        default_value = "stablesats.yml",
        value_name = "FILE"
    )]
    config: PathBuf,

    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Runs the configured processes
    Run {
        /// Output config on crash
        #[clap(env = "CRASH_REPORT_CONFIG")]
        crash_report_config: Option<bool>,
        /// Connection string for the stablesats database
        #[clap(env = "PG_CON", default_value = "")]
        pg_con: String,
        /// API key for the galoy client
        #[clap(env = "GALOY_API_KEY", default_value = "")]
        galoy_api_key: String,
        /// Okex secret key
        #[clap(env = "OKEX_SECRET_KEY", default_value = "")]
        okex_secret_key: String,
        /// Okex passphrase
        #[clap(env = "OKEX_PASSPHRASE", default_value = "")]
        okex_passphrase: String,
        /// Bria profile api key
        #[clap(env = "BRIA_PROFILE_API_KEY", default_value = "")]
        bria_profile_api_key: String,
    },
    /// Gets a quote from the price server
    Price {
        /// price server URL
        #[clap(short, long, action, value_parser, env = "PRICE_SERVER_URL")]
        url: Option<Url>,
        #[clap(short, long, action, value_enum, value_parser, default_value_t = Direction::Buy)]
        direction: Direction,
        /// For option price expiry in seconds
        #[clap(short, long, action)]
        expiry: Option<u64>,
        amount: Decimal,
    },

    /// Gets a quote from the quote serve
    GetQuote {
        /// quote server URL
        #[clap(short, long, action, value_parser, env = "QUOTE_SERVER_URL")]
        url: Option<Url>,
        #[clap(short, long)]
        immediate_execution: bool,
        #[clap(short, long, action, value_enum, value_parser, default_value_t = QuoteDirection::Buy)]
        direction: QuoteDirection,
        #[clap(short, long, action, value_enum, value_parser, default_value_t = Currency::Cents)]
        currency: Currency,
        amount: u64,
    },

    AcceptQuote {
        /// quote server URL
        #[clap(short, long, action, value_parser, env = "QUOTE_SERVER_URL")]
        url: Option<Url>,
        #[clap(short, long)]
        id: String,
    },
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Run {
            crash_report_config,
            galoy_api_key,
            okex_passphrase,
            okex_secret_key,
            pg_con,
            bria_profile_api_key,
        } => {
            let config = Config::from_path(
                cli.config,
                EnvOverride {
                    galoy_api_key,
                    okex_passphrase,
                    okex_secret_key,
                    pg_con,
                    bria_profile_api_key,
                },
            )?;
            match (run_cmd(config.clone()).await, crash_report_config) {
                (Err(e), Some(true)) => {
                    println!("Stablesats was started with the following config:");
                    println!("{}", serde_yaml::to_string(&config).unwrap());
                    return Err(e);
                }
                (Err(e), _) => return Err(e),
                _ => (),
            }
        }
        Command::Price {
            url,
            direction,
            expiry,
            amount,
        } => price_cmd(url, direction, expiry, amount).await?,

        Command::GetQuote {
            url,
            immediate_execution,
            direction,
            currency,
            amount,
        } => {
            let client = get_quotes_client(url).await;
            client
                .get_quote(direction, currency, immediate_execution, amount)
                .await?
        }
        Command::AcceptQuote { url, id } => {
            let client = get_quotes_client(url).await;
            client.accept_quote(id).await?;
        }
    }
    Ok(())
}

async fn run_cmd(
    Config {
        db,
        price_server,
        user_trades,
        tracing,
        galoy,
        hedging,
        exchanges,
        bria,
        quotes_server,
    }: Config,
) -> anyhow::Result<()> {
    println!("Stablesats - v{}", env!("CARGO_PKG_VERSION"));
    println!("Starting server process");
    crate::tracing::init_tracer(tracing)?;

    let (send, mut receive) = tokio::sync::mpsc::channel(1);
    let mut handles = Vec::new();
    let mut checkers = HashMap::new();
    let (price_send, price_recv) = memory::channel(price_stream_throttle_period());

    let unhealthy_msg_interval = price_server
        .health
        .unhealthy_msg_interval_price
        .to_std()
        .expect("Could not convert Duration to_std");
    if exchanges
        .okex
        .as_ref()
        .map(|okex| okex.weight > Decimal::ZERO)
        .unwrap_or(false)
    {
        println!("Starting Okex price feed");

        let okex_send = send.clone();
        let price_send = price_send.clone();
        handles.push(tokio::spawn(async move {
            let _ = okex_send.try_send(
                okex_price::run(price_send, unhealthy_msg_interval / 2)
                    .await
                    .context("Okex Price Feed error"),
            );
        }));
    }

    if price_server.enabled {
        println!(
            "Starting price server on port {}",
            price_server.server.listen_port
        );

        let price_send = send.clone();
        let (snd, recv) = futures::channel::mpsc::unbounded();
        checkers.insert("price", snd);
        let price = price_recv.resubscribe();
        let weights = extract_weights(&exchanges);
        handles.push(tokio::spawn(async move {
            let _ = price_send.try_send(
                price_server::run(
                    recv,
                    price_server.health,
                    price_server.server,
                    price_server.fees,
                    price,
                    price_server.price_cache,
                    weights,
                )
                .await
                .context("Price Server error"),
            );
        }));
    }

    let mut pool = None;
    let mut ledger = None;

    if hedging.enabled {
        println!("Starting hedging process");

        let hedging_send = send.clone();
        let galoy = galoy.clone();
        let bria = bria.clone();
        let (snd, recv) = futures::channel::mpsc::unbounded();
        let price = price_recv.resubscribe();
        checkers.insert("hedging", snd);

        if let Some(okex_cfg) = exchanges.okex.as_ref() {
            pool = Some(crate::db::init_pool(&db).await?);
            ledger = Some(ledger::Ledger::init(pool.as_ref().unwrap()).await?);

            let okex_config = okex_cfg.config.clone();
            let pool = pool.clone();
            let ledger = ledger.clone();
            handles.push(tokio::spawn(async move {
                let _ = hedging_send.try_send(
                    hedging::run(
                        pool.as_ref().unwrap().clone(),
                        recv,
                        hedging.config,
                        okex_config,
                        galoy,
                        bria,
                        price,
                        ledger.as_ref().unwrap().clone(),
                    )
                    .await
                    .context("Hedging error"),
                );
            }));
        }
    }

    if quotes_server.enabled {
        println!("Starting quotes_server");

        if pool.is_none() {
            pool = Some(crate::db::init_pool(&db).await?);
            ledger = Some(ledger::Ledger::init(pool.as_ref().unwrap()).await?);
        }
        let quotes_send = send.clone();
        let (snd, recv) = futures::channel::mpsc::unbounded();
        checkers.insert("quotes", snd);
        let price = price_recv.resubscribe();
        let weights = extract_weights_for_quotes_server(&exchanges);
        let ledger = ledger.clone();
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            let _ = quotes_send.try_send(
                quotes_server::run(
                    pool.as_ref().unwrap().clone(),
                    recv,
                    quotes_server.health,
                    quotes_server.server,
                    quotes_server.fees,
                    price,
                    quotes_server.price_cache,
                    weights,
                    quotes_server.config,
                    ledger.as_ref().unwrap().clone(),
                )
                .await
                .context("Quote Server error"),
            );
        }));
    }

    if user_trades.enabled {
        println!("Starting user trades process");
        if pool.is_none() {
            pool = Some(crate::db::init_pool(&db).await?);
            ledger = Some(ledger::Ledger::init(pool.as_ref().unwrap()).await?);
        }

        let user_trades_send = send.clone();
        handles.push(tokio::spawn(async move {
            let _ = user_trades_send.try_send(
                user_trades::run(pool.unwrap(), user_trades.config, galoy, ledger.unwrap())
                    .await
                    .context("User Trades error"),
            );
        }));
    }

    handles.push(tokio::spawn(async move {
        let _ = send.try_send(crate::health::run(checkers).await);
    }));
    let reason = receive.recv().await.expect("Didn't receive msg");
    for handle in handles {
        handle.abort();
    }
    reason
}

async fn price_cmd(
    url: Option<Url>,
    direction: Direction,
    expiry: Option<u64>,
    amount: Decimal,
) -> anyhow::Result<()> {
    let client = PriceClient::new(url.map(|url| PriceClientConfig { url }).unwrap_or_default());
    client.get_price(direction, expiry, amount).await
}

async fn get_quotes_client(url: Option<Url>) -> QuotesClient {
    QuotesClient::new(
        url.map(|url| QuotesClientConfig { url })
            .unwrap_or_default(),
    )
}

fn price_stream_throttle_period() -> Duration {
    Duration::from_std(std::time::Duration::from_millis(500)).unwrap()
}

fn extract_weights(config: &hedging::ExchangesConfig) -> price_server::ExchangeWeights {
    price_server::ExchangeWeights {
        okex: config.okex.as_ref().map(|c| c.weight),
    }
}

fn extract_weights_for_quotes_server(
    config: &hedging::ExchangesConfig,
) -> quotes_server::ExchangeWeights {
    quotes_server::ExchangeWeights {
        okex: config.okex.as_ref().map(|c| c.weight),
    }
}

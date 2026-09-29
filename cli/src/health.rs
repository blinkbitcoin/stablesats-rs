use anyhow::Context;
use axum::{http::StatusCode, routing::get, Router};
use futures::SinkExt;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::RwLock;
use tracing::{instrument, trace, warn};

use shared::health::HealthChecker;

async fn health_check(
    checkers: Arc<HashMap<&'static str, HealthChecker>>,
    n_errors: Arc<RwLock<usize>>,
) -> StatusCode {
    for (name, checker) in checkers.iter() {
        trace!("Executing '{name}' health check:");
        let (snd, recv) = futures::channel::oneshot::channel();
        if let Err(e) = checker.clone().send(snd).await {
            warn!("Couldn't send '{name}' health check: {e}");
            return health_check_error(name, n_errors, e).await;
        }
        match tokio::time::timeout(std::time::Duration::from_millis(500), recv).await {
            Err(e) => {
                warn!("'{name}' health check timed out");
                return health_check_error(name, n_errors, e).await;
            }
            Ok(Err(e)) => {
                warn!("Error receiving return '{name}' {e}");
                return health_check_error(name, n_errors, e).await;
            }
            Ok(Ok(Err(e))) => {
                warn!("'{name}' FAILED: '{e}'");
                return health_check_error(name, n_errors, e).await;
            }
            _ => {
                trace!("'{name}' health OK");
            }
        }
    }
    let mut n_errors = n_errors.write().await;
    *n_errors = 0;
    StatusCode::OK
}

#[instrument(name = "health.health_check_error", skip_all, fields(component_name, error = true, error.level, error.message, n_errors))]
async fn health_check_error(
    name: &str,
    n_errors: Arc<RwLock<usize>>,
    err: impl std::fmt::Display,
) -> StatusCode {
    let mut n_errors = n_errors.write().await;
    *n_errors += 1;
    let span = tracing::Span::current();
    span.record("component_name", name);
    span.record("n_errors", *n_errors);
    span.record("error.message", tracing::field::display(&err));
    if *n_errors > 4 {
        span.record(
            "error.level",
            tracing::field::display(&tracing::Level::ERROR),
        );
    } else {
        span.record(
            "error.level",
            tracing::field::display(&tracing::Level::WARN),
        );
    }

    StatusCode::SERVICE_UNAVAILABLE
}

pub async fn run(checkers: HashMap<&'static str, HealthChecker>) -> anyhow::Result<()> {
    run_at(SocketAddr::from(([0, 0, 0, 0], 8080)), checkers).await
}

async fn run_at(
    addr: SocketAddr,
    checkers: HashMap<&'static str, HealthChecker>,
) -> anyhow::Result<()> {
    let checkers = Arc::new(checkers);
    let app = Router::new()
        .route(
            "/health/live",
            get({
                let checkers = checkers.clone();
                let n_errors = Arc::new(tokio::sync::RwLock::new(0));
                move || health_check(Arc::clone(&checkers), Arc::clone(&n_errors))
            }),
        )
        .route(
            "/health/startup",
            get({
                let checkers = checkers.clone();
                move || health_check(Arc::clone(&checkers), Arc::new(RwLock::new(0)))
            }),
        )
        .route(
            "/health/ready",
            get({
                let checkers = checkers.clone();
                let ever_ready = Arc::new(RwLock::new(false));
                || async move {
                    let ever_ready = Arc::clone(&ever_ready);
                    if *ever_ready.read().await {
                        StatusCode::OK
                    } else {
                        let ret =
                            health_check(Arc::clone(&checkers), Arc::new(RwLock::new(0))).await;
                        if ret == StatusCode::OK {
                            *ever_ready.write().await = true;
                        }
                        ret
                    }
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Bind health server")?;
    axum::serve(listener, app)
        .await
        .context("Serve health server")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn serves_health_routes_and_reports_bind_errors() -> anyhow::Result<()> {
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = reserved.local_addr()?;
        let error = run_at(addr, HashMap::new()).await.unwrap_err();
        assert_eq!(error.to_string(), "Bind health server");
        drop(reserved);

        let server = tokio::spawn(run_at(addr, HashMap::new()));
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            for route in ["live", "startup", "ready"] {
                let mut stream = loop {
                    match tokio::net::TcpStream::connect(addr).await {
                        Ok(stream) => break stream,
                        Err(_) => tokio::task::yield_now().await,
                    }
                };
                stream
                    .write_all(
                        format!("GET /health/{route} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                            .as_bytes(),
                    )
                    .await?;
                let mut response = String::new();
                stream.read_to_string(&mut response).await?;
                assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
            }
            anyhow::Ok(())
        })
        .await;
        server.abort();
        result??;
        Ok(())
    }
}

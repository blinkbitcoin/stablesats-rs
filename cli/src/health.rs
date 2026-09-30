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
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Bind health server")?;
    axum::serve(listener, health_router(checkers))
        .await
        .context("Serve health server")
}

fn health_router(checkers: HashMap<&'static str, HealthChecker>) -> Router {
    let checkers = Arc::new(checkers);
    Router::new()
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
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    async fn request_status(app: Router, route: &str) -> StatusCode {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            app.oneshot(
                axum::http::Request::builder()
                    .uri(format!("/health/{route}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
        .unwrap()
        .status()
    }

    #[tokio::test]
    async fn health_routes_handle_failures_recovery_and_sticky_readiness() {
        let (checker, mut trigger) = futures::channel::mpsc::unbounded();
        let mode = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_mode = mode.clone();
        let worker_calls = calls.clone();
        let worker = tokio::spawn(async move {
            let mut pending = Vec::new();
            while let Some(response) = trigger.next().await {
                let response: futures::channel::oneshot::Sender<Result<(), String>> = response;
                worker_calls.fetch_add(1, Ordering::SeqCst);
                match worker_mode.load(Ordering::SeqCst) {
                    0 => {
                        let _ = response.send(Err("unhealthy".into()));
                    }
                    1 => {
                        let _ = response.send(Ok(()));
                    }
                    2 => drop(response),
                    3 => pending.push(response), // Keep the sender alive to exercise timeout.
                    _ => unreachable!(),
                }
            }
        });
        let app = health_router(HashMap::from([("test", checker)]));
        for route in ["ready", "startup", "live", "live", "live", "live", "live"] {
            assert_eq!(
                request_status(app.clone(), route).await,
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        mode.store(1, Ordering::SeqCst);
        for route in ["live", "startup", "ready"] {
            assert_eq!(request_status(app.clone(), route).await, StatusCode::OK);
        }
        for failure_mode in [0, 2, 3] {
            mode.store(failure_mode, Ordering::SeqCst);
            assert_eq!(
                request_status(app.clone(), "live").await,
                StatusCode::SERVICE_UNAVAILABLE
            );
            let calls_before = calls.load(Ordering::SeqCst);
            assert_eq!(request_status(app.clone(), "ready").await, StatusCode::OK);
            assert_eq!(
                calls.load(Ordering::SeqCst),
                calls_before,
                "readiness must stay successful without rechecking"
            );
        }
        worker.abort();
        worker.await.unwrap_err();
        assert_eq!(
            request_status(app.clone(), "live").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(request_status(app, "ready").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn successful_check_resets_consecutive_errors() {
        let (checker, mut trigger) = futures::channel::mpsc::unbounded();
        let worker = tokio::spawn(async move {
            for outcome in [Err("failed".into()), Err("failed".into()), Ok(())] {
                let response: futures::channel::oneshot::Sender<Result<(), String>> =
                    trigger.next().await.unwrap();
                response.send(outcome).unwrap();
            }
        });
        let checkers = Arc::new(HashMap::from([("test", checker)]));
        let errors = Arc::new(RwLock::new(0));
        for expected in [1, 2] {
            assert_eq!(
                health_check(checkers.clone(), errors.clone()).await,
                StatusCode::SERVICE_UNAVAILABLE
            );
            assert_eq!(*errors.read().await, expected);
        }
        assert_eq!(health_check(checkers, errors.clone()).await, StatusCode::OK);
        assert_eq!(*errors.read().await, 0);
        worker.await.unwrap();
    }

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

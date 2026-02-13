//! HTTP server for axum with dpdk-net transport.
//!
//! Provides [`serve`] for running an axum [`Router`] on a dpdk-net
//! [`TcpListener`], bypassing `axum::serve()` to avoid `Send` bounds.
//!
//! # Why not `axum::serve()`?
//!
//! dpdk-net sockets use `Rc<RefCell<...>>` internally, making them `!Send`.
//! Standard `axum::serve()` requires `Send` streams. This module uses
//! hyper-util's [`AutoBuilder`] with a [`LocalExecutor`] that spawns tasks
//! via `tokio::task::spawn_local` instead of `tokio::spawn`.
//!
//! [`AutoBuilder`]: hyper_util::server::conn::auto::Builder

use axum::Router;
use dpdk_net::runtime::tokio_compat::TokioTcpStream;
use dpdk_net::socket::TcpListener;
use dpdk_net_hyper::LocalExecutor;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use hyper_util::service::TowerToHyperService;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

/// Serve an axum [`Router`] on a dpdk-net [`TcpListener`].
///
/// Accepts connections and serves each one using hyper's auto-detection
/// (HTTP/1.1 or HTTP/2 cleartext). Uses [`LocalExecutor`] to handle
/// `!Send` futures from dpdk-net's socket types.
///
/// Runs until the `shutdown` token is cancelled.
///
/// # Example
///
/// ```ignore
/// use dpdk_net_axum::{DpdkApp, WorkerContext, serve};
/// use dpdk_net::socket::TcpListener;
/// use axum::{Router, routing::get};
/// use smoltcp::wire::Ipv4Address;
///
/// let app = Router::new().route("/", get(|| async { "Hello!" }));
///
/// DpdkApp::new()
///     .eth_dev(0)
///     .ip(Ipv4Address::new(10, 0, 0, 10))
///     .gateway(Ipv4Address::new(10, 0, 0, 1))
///     .run(shutdown, move |ctx: WorkerContext| {
///         let app = app.clone();
///         async move {
///             let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
///             serve(listener, app, ctx.shutdown).await;
///         }
///     });
/// ```
pub async fn serve(mut listener: TcpListener, app: Router, shutdown: CancellationToken) {
    info!("Axum server starting");
    let mut conn_id = 0u64;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            result = listener.accept() => {
                match result {
                    Ok(stream) => {
                        let id = conn_id;
                        conn_id += 1;
                        debug!(conn_id = id, "Connection accepted");

                        let app = app.clone();
                        let io = TokioIo::new(TokioTcpStream::new(stream));

                        tokio::task::spawn_local(async move {
                            let result = AutoBuilder::new(LocalExecutor)
                                .serve_connection(io, TowerToHyperService::new(app))
                                .await;

                            match result {
                                Ok(()) => debug!(conn_id = id, "Connection closed"),
                                Err(e) => debug!(conn_id = id, error = %e, "Connection error"),
                            }
                        });
                    }
                    Err(e) => {
                        error!(error = ?e, "Accept failed");
                    }
                }
            }
        }
    }

    info!("Axum server stopped");
}

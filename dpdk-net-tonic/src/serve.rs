//! gRPC server for dpdk-net.
//!
//! Thin wrapper over [`dpdk_net_axum::serve`] that accepts tonic's [`Routes`]
//! directly, so gRPC users don't need to depend on axum types.

use dpdk_net::socket::TcpListener;
use tonic::service::Routes;
use tracing::info;

use std::future::Future;

/// Serve tonic gRPC services on a dpdk-net [`TcpListener`].
///
/// Converts tonic's [`Routes`] to an `axum::Router` via
/// `.into_axum_router()` and delegates to [`dpdk_net_axum::serve`].
///
/// Runs until the `shutdown` future completes.
pub async fn serve(listener: TcpListener, routes: Routes, shutdown: impl Future<Output = ()>) {
    info!("Tonic gRPC server starting");
    let axum_router = routes.into_axum_router();
    dpdk_net_axum::serve(listener, axum_router, shutdown).await;
}

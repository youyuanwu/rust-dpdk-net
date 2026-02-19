//! Worker context passed to each lcore.

use dpdk_net::api::rte::lcore::Lcore;
use dpdk_net::runtime::ReactorHandle;

/// Context passed to each worker lcore.
///
/// This provides everything needed to run a server or client on a specific lcore:
/// - Access to the lcore information (ID, socket, etc.)
/// - Reactor handle for creating TCP/UDP sockets
///
/// # Example
///
/// ```ignore
/// use dpdk_net_util::WorkerContext;
/// use dpdk_net::socket::TcpListener;
///
/// async fn my_server(ctx: WorkerContext) {
///     // Create a server listener
///     let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
///     // ... serve requests
/// }
/// ```
pub struct WorkerContext {
    /// The lcore this worker is running on.
    pub lcore: Lcore,

    /// Queue ID (0 = main lcore, 1+ = workers).
    ///
    /// This matches the lcore index in the order returned by `Lcore::all()`.
    pub queue_id: u16,

    /// NUMA socket ID for this lcore.
    ///
    /// Useful for NUMA-aware memory allocation.
    pub socket_id: u32,

    /// Reactor handle for creating sockets.
    ///
    /// Use this to create `TcpListener` (server) or `TcpStream` (client).
    pub reactor: ReactorHandle,
}

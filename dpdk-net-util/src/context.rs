//! Worker context passed to each lcore.

use dpdk_net::api::rte::lcore::Lcore;
use dpdk_net::runtime::ReactorHandle;
use dpdk_net::socket::{TcpListener, UdpSocket};
use smoltcp::socket::tcp::ListenError;
use smoltcp::socket::udp::BindError as UdpBindError;
use std::cell::Cell;

/// Start of the ephemeral port range.
const EPHEMERAL_PORT_START: u16 = 32768;
/// End of the ephemeral port range (inclusive).
const EPHEMERAL_PORT_END: u16 = 60999;

/// Context passed to each worker lcore.
///
/// This provides everything needed to run a server or client on a specific lcore:
/// - Access to the lcore information (ID, socket, etc.)
/// - Reactor handle for creating TCP/UDP sockets
/// - Ephemeral port allocator for client connections
///
/// # Example
///
/// ```ignore
/// use dpdk_net_util::WorkerContext;
/// use dpdk_net::socket::TcpListener;
///
/// async fn my_server(ctx: WorkerContext) {
///     // Create a server listener with default buffer sizes
///     let listener = ctx.bind_tcp(8080).unwrap();
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

    /// Per-lcore ephemeral port counter (starts offset by queue_id to avoid collisions).
    next_ephemeral_port: Cell<u16>,
}

impl WorkerContext {
    /// Create a new WorkerContext.
    pub(crate) fn new(lcore: Lcore, queue_id: u16, socket_id: u32, reactor: ReactorHandle) -> Self {
        // Offset starting port by queue_id to reduce inter-queue collisions
        let start = EPHEMERAL_PORT_START + (queue_id as u16 % 100) * 256;
        Self {
            lcore,
            queue_id,
            socket_id,
            reactor,
            next_ephemeral_port: Cell::new(start),
        }
    }

    /// Allocate the next ephemeral port for a client TCP connection.
    ///
    /// Each lcore has its own counter, starting at a queue-specific offset
    /// to reduce collisions. Wraps around within the ephemeral range.
    pub fn alloc_ephemeral_port(&self) -> u16 {
        let port = self.next_ephemeral_port.get();
        let next = if port >= EPHEMERAL_PORT_END {
            EPHEMERAL_PORT_START
        } else {
            port + 1
        };
        self.next_ephemeral_port.set(next);
        port
    }

    /// Bind a TCP listener on the given port with default buffer sizes (16KB rx/tx, backlog 2).
    pub fn bind_tcp(&self, port: u16) -> Result<TcpListener, ListenError> {
        TcpListener::bind(&self.reactor, port, 16384, 16384)
    }

    /// Bind a UDP socket on the given port with default buffer sizes (64 packets, 1536 bytes each).
    pub fn bind_udp(&self, port: u16) -> Result<UdpSocket, UdpBindError> {
        UdpSocket::bind(&self.reactor, port, 64, 64, 1536)
    }

    /// Bind a UDP socket with custom buffer sizes.
    pub fn bind_udp_with_buffers(
        &self,
        port: u16,
        rx_buffer_packets: usize,
        tx_buffer_packets: usize,
        max_packet_size: usize,
    ) -> Result<UdpSocket, UdpBindError> {
        UdpSocket::bind(
            &self.reactor,
            port,
            rx_buffer_packets,
            tx_buffer_packets,
            max_packet_size,
        )
    }
}

//! Reusable async TCP echo server components for DPDK + smoltcp.
//!
//! This module provides building blocks for creating TCP echo servers
//! that can be used in tests and examples.
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net_test::app::echo_server::{EchoServer, ServerStats};
//! use dpdk_net::socket::TcpListener;
//! use dpdk_net::runtime::ReactorHandle;
//! use tokio_util::sync::CancellationToken;
//! use std::sync::Arc;
//!
//! async fn run(listener: TcpListener, cancel: CancellationToken) {
//!     let stats = Arc::new(ServerStats::default());
//!     let server = EchoServer::new(listener, cancel, stats, 0, 8080);
//!     server.run().await;
//! }
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dpdk_net::socket::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

/// Statistics for the echo server.
///
/// All fields use atomic operations for thread-safe access.
#[derive(Default)]
pub struct ServerStats {
    /// Total number of connections accepted
    pub connections: AtomicU64,
    /// Total bytes received across all connections
    pub bytes_received: AtomicU64,
    /// Total bytes sent across all connections
    pub bytes_sent: AtomicU64,
    /// Number of send errors encountered
    pub send_errors: AtomicU64,
}

impl ServerStats {
    /// Create a new statistics tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the current connection count.
    pub fn connections(&self) -> u64 {
        self.connections.load(Ordering::Relaxed)
    }

    /// Get the current bytes received count.
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received.load(Ordering::Relaxed)
    }

    /// Get the current bytes sent count.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Get the current send errors count.
    pub fn send_errors(&self) -> u64 {
        self.send_errors.load(Ordering::Relaxed)
    }

    /// Print a summary of the statistics.
    pub fn print_summary(&self, runtime_secs: u64) {
        info!(
            runtime_secs,
            connections = self.connections(),
            bytes_received = self.bytes_received(),
            bytes_sent = self.bytes_sent(),
            send_errors = self.send_errors(),
            "Server statistics"
        );
    }
}

/// Handle a single client connection: receive and echo data until closed.
///
/// This function reads data from the stream and echoes it back until
/// the client closes the connection or an error occurs.
pub async fn handle_connection(stream: TcpStream, conn_id: u64, stats: Arc<ServerStats>) {
    let mut buf = [0u8; 4096];

    loop {
        // Receive data
        let len = match stream.recv(&mut buf).await {
            Ok(0) => {
                debug!(conn_id, "Client closed connection");
                break;
            }
            Ok(len) => len,
            Err(e) => {
                error!(conn_id, error = ?e, "Recv error");
                break;
            }
        };

        stats
            .bytes_received
            .fetch_add(len as u64, Ordering::Relaxed);

        // Echo it back
        match stream.send(&buf[..len]).await {
            Ok(len) => {
                stats.bytes_sent.fetch_add(len as u64, Ordering::Relaxed);
            }
            Err(e) => {
                error!(conn_id, error = ?e, "Send error");
                stats.send_errors.fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
    }

    // Close gracefully
    stream.close().await.ok();
}

/// Async TCP echo server.
///
/// Accepts connections and spawns handlers that echo data back to clients.
pub struct EchoServer {
    listener: TcpListener,
    cancel: CancellationToken,
    stats: Arc<ServerStats>,
    queue_id: usize,
    port: u16,
}

impl EchoServer {
    /// Create a new echo server.
    ///
    /// # Arguments
    /// * `listener` - The TCP listener to accept connections on
    /// * `cancel` - Cancellation token for graceful shutdown
    /// * `stats` - Shared statistics tracker
    /// * `queue_id` - Queue identifier for logging
    /// * `port` - Port number for logging
    pub fn new(
        listener: TcpListener,
        cancel: CancellationToken,
        stats: Arc<ServerStats>,
        queue_id: usize,
        port: u16,
    ) -> Self {
        Self {
            listener,
            cancel,
            stats,
            queue_id,
            port,
        }
    }

    /// Run the server until cancellation.
    ///
    /// This accepts connections in a loop and spawns a handler task for each.
    /// Returns when the cancellation token is triggered.
    pub async fn run(mut self) {
        info!(queue_id = self.queue_id, port = self.port, "Listening");

        let mut conn_id = 0u64;

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    break;
                }
                result = self.listener.accept() => {
                    match result {
                        Ok(stream) => {
                            let id = conn_id;
                            conn_id += 1;
                            self.stats.connections.fetch_add(1, Ordering::Relaxed);
                            debug!(queue_id = self.queue_id, conn_id = id, "Connection accepted");

                            // Spawn handler as background task
                            let stats_clone = self.stats.clone();
                            tokio::task::spawn_local(async move {
                                handle_connection(stream, id, stats_clone).await;
                            });
                        }
                        Err(e) => {
                            error!(queue_id = self.queue_id, error = ?e, "Accept error");
                        }
                    }
                }
            }
        }

        info!(queue_id = self.queue_id, "Shutting down");
    }
}

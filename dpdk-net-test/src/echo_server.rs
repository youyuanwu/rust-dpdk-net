//! TCP Echo Server loop logic
//!
//! This module provides a long-running server loop for TCP echo servers with
//! verbose logging, status messages, and graceful shutdown support.
//!
//! Uses `tcp_echo::EchoServer` internally for the echo logic.

use crate::tcp_echo::{EchoServer, SocketConfig};
use smoltcp::iface::{Interface, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::tcp::State;
use smoltcp::time::Instant;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Configuration for the echo server
pub struct EchoServerConfig {
    /// How often to print status messages (in seconds)
    pub status_interval_secs: u64,
    /// How often to print "waiting for connection" messages (in loop iterations)
    pub waiting_message_interval: u64,
    /// Sleep duration between poll iterations
    pub poll_interval: Duration,
    /// Maximum size for the pending TX buffer (0 = unlimited)
    pub max_pending_tx_bytes: usize,
}

impl Default for EchoServerConfig {
    fn default() -> Self {
        Self {
            status_interval_secs: 10,
            waiting_message_interval: 10000,
            poll_interval: Duration::from_micros(100),
            max_pending_tx_bytes: 0, // unlimited
        }
    }
}

/// Statistics from the echo server run
#[derive(Debug, Default, Clone)]
pub struct EchoServerStats {
    /// Total bytes received
    pub bytes_received: u64,
    /// Total bytes sent
    pub bytes_sent: u64,
    /// Number of connections handled
    pub connections: u64,
    /// Number of send errors
    pub send_errors: u64,
    /// Bytes dropped due to buffer overflow or connection close
    pub bytes_dropped: u64,
}

/// Result of running the echo server
pub struct EchoServerResult {
    /// Statistics from the run
    pub stats: EchoServerStats,
    /// Total runtime in seconds
    pub runtime_secs: u64,
}

/// Format current time as HH:MM:SS
pub fn format_time() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let secs = now.as_secs() % 86400; // Seconds since midnight
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

/// Run the TCP echo server loop
///
/// This function runs the main server loop with verbose logging, polling the
/// network interface and using `tcp_echo::EchoServer` for echo logic.
///
/// # Arguments
/// * `device` - The network device (must implement smoltcp's Device trait)
/// * `iface` - The configured smoltcp interface
/// * `sockets` - The socket set (server socket will be added)
/// * `port` - Port to listen on
/// * `running` - Atomic bool to control shutdown (set to false to stop)
/// * `config` - Server configuration
///
/// # Returns
/// Returns `EchoServerResult` with statistics when the server stops.
pub fn run_echo_server<D: Device>(
    device: &mut D,
    iface: &mut Interface,
    sockets: &mut SocketSet<'_>,
    port: u16,
    running: Arc<AtomicBool>,
    config: EchoServerConfig,
) -> EchoServerResult {
    // Create the echo server using tcp_echo
    let socket_config = if config.max_pending_tx_bytes > 0 {
        SocketConfig {
            rx_buffer_size: config.max_pending_tx_bytes,
            tx_buffer_size: config.max_pending_tx_bytes,
        }
    } else {
        SocketConfig::default()
    };
    let mut server = EchoServer::new(sockets, port, socket_config);

    let mut iteration = 0u64;
    let start_time = std::time::Instant::now();
    let mut last_status_time = start_time;
    let mut was_connected = false;
    let mut last_stats = server.stats().clone();

    println!("[{}] Echo server listening on port {}", format_time(), port);

    while running.load(Ordering::SeqCst) {
        let timestamp = Instant::now();
        iface.poll(timestamp, device, sockets);

        let state = server.state(sockets);
        let is_connected = server.is_connected(sockets);

        if is_connected {
            // Track new connections
            if !was_connected {
                println!("[{}] New connection established", format_time());
                was_connected = true;
            }

            // Print connection status periodically
            let now = std::time::Instant::now();
            if now.duration_since(last_status_time).as_secs() >= config.status_interval_secs {
                let stats = server.stats();
                println!(
                    "[{}] Connection active (uptime: {}s, rx: {} bytes, tx: {} bytes)",
                    format_time(),
                    start_time.elapsed().as_secs(),
                    stats.bytes_received,
                    stats.bytes_sent
                );
                last_status_time = now;
            }

            // Process echo (receive and send)
            let echoed = server.process(sockets);

            // Log received/sent data
            let stats = server.stats();
            if stats.bytes_received > last_stats.bytes_received {
                let received = stats.bytes_received - last_stats.bytes_received;
                println!("[{}] [RX] {} bytes", format_time(), received);
            }
            if echoed > 0 {
                println!("[{}] [TX] Echoed {} bytes", format_time(), echoed);
            }
            last_stats = stats.clone();
        } else {
            // Process to handle re-listen
            server.process(sockets);

            // Connection closed
            if was_connected {
                println!("[{}] Connection closed", format_time());
                was_connected = false;
            }

            if iteration.is_multiple_of(config.waiting_message_interval) && state == State::Listen {
                println!(
                    "[{}] Waiting for connections... (uptime: {}s)",
                    format_time(),
                    start_time.elapsed().as_secs()
                );
            }
        }

        iteration += 1;
        std::thread::sleep(config.poll_interval);
    }

    let stats = server.stats();
    EchoServerResult {
        stats: EchoServerStats {
            bytes_received: stats.bytes_received as u64,
            bytes_sent: stats.bytes_sent as u64,
            connections: stats.connections as u64,
            send_errors: 0,
            bytes_dropped: 0,
        },
        runtime_secs: start_time.elapsed().as_secs(),
    }
}

/// Setup Ctrl+C handler that sets the running flag to false
///
/// # Returns
/// Returns an Arc<AtomicBool> that will be set to false when Ctrl+C is pressed
pub fn setup_ctrlc_handler() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("\n\nReceived Ctrl+C, shutting down...");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");
    running
}

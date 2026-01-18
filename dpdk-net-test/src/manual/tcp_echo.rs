//! TCP Echo Server and Client
//!
//! Provides reusable `EchoServer` and `EchoClient` abstractions for testing
//! TCP echo functionality with smoltcp.
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net_test::tcp_echo::{EchoServer, EchoClient, SocketConfig, run_echo_test};
//! use dpdk_net::tcp::DpdkDeviceWithPool;
//! use smoltcp::iface::{Interface, SocketSet};
//! use smoltcp::time::Instant;
//! use smoltcp::wire::Ipv4Address;
//! use std::time::Duration;
//!
//! fn example(
//!     device: &mut DpdkDeviceWithPool,
//!     iface: &mut Interface,
//!     sockets: &mut SocketSet<'static>,
//! ) {
//!     let server_ip = Ipv4Address::new(192, 168, 1, 1);
//!     let server_port = 8080;
//!
//!     // Create server and client
//!     let mut server = EchoServer::new(sockets, server_port, SocketConfig::default());
//!     let mut client = EchoClient::new(
//!         sockets, iface, server_ip, server_port, 49152, SocketConfig::default(),
//!     );
//!     client.send(b"Hello!");
//!
//!     // Poll until client receives echo
//!     while !client.is_complete() {
//!         iface.poll(Instant::now(), device, sockets);
//!         server.process(sockets);
//!         client.process(sockets);
//!     }
//! }
//! ```

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::tcp::{self, State};
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, Ipv4Address};
use std::time::Duration;

/// Configuration for creating TCP sockets
#[derive(Debug, Clone)]
pub struct SocketConfig {
    /// Size of the receive buffer
    pub rx_buffer_size: usize,
    /// Size of the transmit buffer
    pub tx_buffer_size: usize,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            rx_buffer_size: 4096,
            tx_buffer_size: 4096,
        }
    }
}

impl SocketConfig {
    pub fn new(rx_size: usize, tx_size: usize) -> Self {
        Self {
            rx_buffer_size: rx_size,
            tx_buffer_size: tx_size,
        }
    }

    pub fn large() -> Self {
        Self {
            rx_buffer_size: 8192,
            tx_buffer_size: 8192,
        }
    }
}

/// Statistics for the echo server
#[derive(Debug, Default, Clone)]
pub struct EchoServerStats {
    pub bytes_received: usize,
    pub bytes_sent: usize,
    pub connections: usize,
}

/// TCP Echo Server that receives data and echoes it back
pub struct EchoServer {
    handle: SocketHandle,
    port: u16,
    pending_tx: Vec<u8>,
    stats: EchoServerStats,
    was_connected: bool,
}

impl EchoServer {
    /// Create a new echo server listening on the specified port
    pub fn new(sockets: &mut SocketSet<'_>, port: u16, config: SocketConfig) -> Self {
        let rx_buffer = tcp::SocketBuffer::new(vec![0; config.rx_buffer_size]);
        let tx_buffer = tcp::SocketBuffer::new(vec![0; config.tx_buffer_size]);
        let mut socket = tcp::Socket::new(rx_buffer, tx_buffer);
        socket.listen(port).expect("Failed to listen on port");
        let handle = sockets.add(socket);

        Self {
            handle,
            port,
            pending_tx: Vec::new(),
            stats: EchoServerStats::default(),
            was_connected: false,
        }
    }

    /// Get the socket handle
    pub fn handle(&self) -> SocketHandle {
        self.handle
    }

    /// Get the listening port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get current statistics
    pub fn stats(&self) -> &EchoServerStats {
        &self.stats
    }

    /// Check if the server is currently connected to a client
    pub fn is_connected(&self, sockets: &SocketSet<'_>) -> bool {
        let socket = sockets.get::<tcp::Socket>(self.handle);
        socket.is_active()
    }

    /// Get the current socket state
    pub fn state(&self, sockets: &SocketSet<'_>) -> State {
        let socket = sockets.get::<tcp::Socket>(self.handle);
        socket.state()
    }

    /// Process the server: receive data and echo it back
    ///
    /// Returns the number of bytes echoed in this call
    pub fn process(&mut self, sockets: &mut SocketSet<'_>) -> usize {
        let socket = sockets.get_mut::<tcp::Socket>(self.handle);
        let mut echoed = 0;

        if socket.is_active() {
            // Track new connections
            if !self.was_connected {
                self.stats.connections += 1;
                self.was_connected = true;
            }

            // Receive data
            if socket.can_recv()
                && let Ok(data) = socket.recv(|buffer| {
                    let len = buffer.len();
                    if len > 0 {
                        (len, buffer.to_vec())
                    } else {
                        (0, vec![])
                    }
                })
                && !data.is_empty()
            {
                self.stats.bytes_received += data.len();
                self.pending_tx.extend_from_slice(&data);
            }

            // Send pending data (echo)
            if !self.pending_tx.is_empty()
                && socket.can_send()
                && let Ok(sent) = socket.send_slice(&self.pending_tx)
            {
                self.stats.bytes_sent += sent;
                echoed = sent;
                self.pending_tx.drain(..sent);
            }

            // If server is in CloseWait (client closed), close the server side too
            // Note: is_active() returns true for CloseWait, so we check explicitly
            if socket.state() == State::CloseWait {
                socket.close();
            }
        } else {
            // Connection closed
            if self.was_connected {
                self.was_connected = false;
                self.pending_tx.clear();
            }

            // Re-listen when Closed
            if socket.state() == State::Closed {
                let _ = socket.listen(self.port);
            }
        }

        echoed
    }

    /// Check if server is ready to accept a new connection
    pub fn is_ready(&self, sockets: &SocketSet<'_>) -> bool {
        let socket = sockets.get::<tcp::Socket>(self.handle);
        socket.state() == State::Listen
    }

    /// Remove the server socket from the socket set
    ///
    /// Call this when you're done with the server to free the socket slot.
    pub fn remove(self, sockets: &mut SocketSet<'_>) {
        sockets.remove(self.handle);
    }
}

/// Statistics for the echo client
#[derive(Debug, Default, Clone)]
pub struct EchoClientStats {
    pub bytes_sent: usize,
    pub bytes_received: usize,
}

/// State of the echo client
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoClientState {
    /// Waiting for connection to be established
    Connecting,
    /// Connected and ready to send
    Connected,
    /// Waiting to receive echo response
    WaitingForEcho,
    /// All expected data received
    Complete,
    /// Connection closed or error
    Closed,
}

/// TCP Echo Client that connects, sends data, and verifies echo
pub struct EchoClient {
    handle: SocketHandle,
    state: EchoClientState,
    data_to_send: Vec<u8>,
    send_offset: usize,
    received_data: Vec<u8>,
    expected_len: usize,
    stats: EchoClientStats,
}

impl EchoClient {
    /// Create a new echo client and initiate connection
    pub fn new(
        sockets: &mut SocketSet<'_>,
        iface: &mut Interface,
        server_ip: Ipv4Address,
        server_port: u16,
        local_port: u16,
        config: SocketConfig,
    ) -> Self {
        let rx_buffer = tcp::SocketBuffer::new(vec![0; config.rx_buffer_size]);
        let tx_buffer = tcp::SocketBuffer::new(vec![0; config.tx_buffer_size]);
        let socket = tcp::Socket::new(rx_buffer, tx_buffer);
        let handle = sockets.add(socket);

        // Initiate connection
        {
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            socket
                .connect(
                    iface.context(),
                    (IpAddress::Ipv4(server_ip), server_port),
                    local_port,
                )
                .expect("Failed to initiate connection");
        }

        Self {
            handle,
            state: EchoClientState::Connecting,
            data_to_send: Vec::new(),
            send_offset: 0,
            received_data: Vec::new(),
            expected_len: 0,
            stats: EchoClientStats::default(),
        }
    }

    /// Get the socket handle
    pub fn handle(&self) -> SocketHandle {
        self.handle
    }

    /// Get current state
    pub fn state(&self) -> EchoClientState {
        self.state
    }

    /// Get current statistics
    pub fn stats(&self) -> &EchoClientStats {
        &self.stats
    }

    /// Check if the client has completed (received all expected data)
    pub fn is_complete(&self) -> bool {
        self.state == EchoClientState::Complete
    }

    /// Check if the client is closed
    pub fn is_closed(&self) -> bool {
        self.state == EchoClientState::Closed
    }

    /// Get the data received so far
    pub fn received_data(&self) -> &[u8] {
        &self.received_data
    }

    /// Queue data to be sent and echoed back
    pub fn send(&mut self, data: &[u8]) {
        self.data_to_send.extend_from_slice(data);
        self.expected_len += data.len();
    }

    /// Verify that received data matches what was sent
    pub fn verify_echo(&self) -> bool {
        // Only compare up to expected_len in case we sent in chunks
        if self.received_data.len() < self.expected_len {
            return false;
        }
        // Compare the data (note: if sent in multiple chunks, order matters)
        self.received_data[..self.expected_len] == self.data_to_send[..self.expected_len]
    }

    /// Process the client: handle connection, send data, receive echo
    ///
    /// Returns true if state changed
    pub fn process(&mut self, sockets: &mut SocketSet<'_>) -> bool {
        let socket = sockets.get_mut::<tcp::Socket>(self.handle);
        let old_state = self.state;

        match self.state {
            EchoClientState::Connecting => {
                if socket.is_active() {
                    self.state = if self.data_to_send.is_empty() {
                        EchoClientState::Connected
                    } else {
                        // Have data queued, try to send
                        EchoClientState::Connected
                    };
                }
            }
            EchoClientState::Connected => {
                // Send any pending data
                if self.send_offset < self.data_to_send.len() && socket.can_send() {
                    let remaining = &self.data_to_send[self.send_offset..];
                    if let Ok(sent) = socket.send_slice(remaining) {
                        self.stats.bytes_sent += sent;
                        self.send_offset += sent;
                    }
                }

                // Transition to waiting if all data sent
                if self.send_offset >= self.data_to_send.len() && !self.data_to_send.is_empty() {
                    self.state = EchoClientState::WaitingForEcho;
                }
            }
            EchoClientState::WaitingForEcho => {
                // Receive echoed data
                if socket.can_recv()
                    && let Ok(data) = socket.recv(|buffer| {
                        let len = buffer.len();
                        if len > 0 {
                            (len, buffer.to_vec())
                        } else {
                            (0, vec![])
                        }
                    })
                    && !data.is_empty()
                {
                    self.stats.bytes_received += data.len();
                    self.received_data.extend_from_slice(&data);
                }

                // Check if complete
                if self.received_data.len() >= self.expected_len {
                    self.state = EchoClientState::Complete;
                }
            }
            EchoClientState::Complete | EchoClientState::Closed => {
                // Nothing to do
            }
        }

        // Check for connection close
        if !socket.is_open() && self.state != EchoClientState::Complete {
            self.state = EchoClientState::Closed;
        }

        self.state != old_state
    }

    /// Close the client connection
    pub fn close(&self, sockets: &mut SocketSet<'_>) {
        let socket = sockets.get_mut::<tcp::Socket>(self.handle);
        socket.close();
    }

    /// Remove the client socket from the socket set
    ///
    /// Call this when you're done with the client to free the socket slot.
    /// The socket should be closed first.
    pub fn remove(self, sockets: &mut SocketSet<'_>) {
        sockets.remove(self.handle);
    }
}

/// Helper to run echo test with server and client in the same interface
pub struct EchoTestResult {
    pub connected: bool,
    pub bytes_sent: usize,
    pub bytes_received: usize,
    pub echo_verified: bool,
}

/// Run a simple echo test: client connects, sends data, verifies echo
pub fn run_echo_test<D: Device>(
    device: &mut D,
    iface: &mut Interface,
    sockets: &mut SocketSet<'_>,
    server: &mut EchoServer,
    client: &mut EchoClient,
    timeout: Duration,
) -> EchoTestResult {
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        let timestamp = Instant::now();
        iface.poll(timestamp, device, sockets);

        server.process(sockets);
        client.process(sockets);

        if client.is_complete() {
            break;
        }

        if client.is_closed() {
            break;
        }

        std::thread::sleep(Duration::from_micros(100));
    }

    EchoTestResult {
        connected: client.state() != EchoClientState::Connecting,
        bytes_sent: client.stats().bytes_sent,
        bytes_received: client.stats().bytes_received,
        echo_verified: client.verify_echo(),
    }
}

/// Run multiple rounds of echo tests (stress test)
#[allow(clippy::too_many_arguments)]
pub fn run_stress_test<D: Device>(
    device: &mut D,
    iface: &mut Interface,
    sockets: &mut SocketSet<'_>,
    server: &mut EchoServer,
    server_ip: Ipv4Address,
    num_rounds: usize,
    messages_per_round: usize,
    round_timeout: Duration,
) -> (bool, Vec<EchoTestResult>) {
    let mut results = Vec::with_capacity(num_rounds);
    let mut all_passed = true;

    for round in 0..num_rounds {
        // Create client for this round
        let local_port = 49152 + round as u16;
        let mut client = EchoClient::new(
            sockets,
            iface,
            server_ip,
            server.port(),
            local_port,
            SocketConfig::default(),
        );

        // Queue messages
        for msg_num in 0..messages_per_round {
            let msg = format!("Round{}-Msg{}", round, msg_num);
            client.send(msg.as_bytes());
        }

        // Run the test - this also polls server back to ready state
        let result = run_echo_test(device, iface, sockets, server, &mut client, round_timeout);

        if !result.echo_verified {
            all_passed = false;
            println!(
                "[Round {}] FAILED - sent: {}, received: {}",
                round, result.bytes_sent, result.bytes_received
            );
        } else {
            println!("[Round {}] PASSED", round);
        }

        // Close client and poll until both sides complete the close handshake
        // and server is ready for the next connection
        client.close(sockets);

        let close_start = std::time::Instant::now();
        let close_timeout = Duration::from_secs(2);
        while close_start.elapsed() < close_timeout {
            let timestamp = Instant::now();
            iface.poll(timestamp, device, sockets);

            server.process(sockets);
            client.process(sockets);

            // Done when server is ready for new connections
            if server.is_ready(sockets) {
                break;
            }

            std::thread::sleep(Duration::from_micros(100));
        }

        client.remove(sockets);
        results.push(result);
    }

    (all_passed, results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_config_default() {
        let config = SocketConfig::default();
        assert_eq!(config.rx_buffer_size, 4096);
        assert_eq!(config.tx_buffer_size, 4096);
    }

    #[test]
    fn test_socket_config_large() {
        let config = SocketConfig::large();
        assert_eq!(config.rx_buffer_size, 8192);
        assert_eq!(config.tx_buffer_size, 8192);
    }
}

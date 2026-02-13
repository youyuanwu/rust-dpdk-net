//! Standalone helper functions for establishing HTTP connections.
//!
//! These are thin wrappers around [`Connection::http1`] and [`Connection::http2`]
//! for callers that prefer a free-function API.

use crate::connection::Connection;
use crate::error::Error;
use dpdk_net::runtime::ReactorHandle;
use smoltcp::wire::IpAddress;

/// Create an HTTP/1.1 connection to the given address.
///
/// Convenience wrapper around [`Connection::http1`].
///
/// # Arguments
/// * `reactor`    – reactor handle for this lcore
/// * `addr`       – remote IP address
/// * `port`       – remote port
/// * `local_port` – ephemeral source port
/// * `rx_buffer`  – TCP receive buffer size in bytes
/// * `tx_buffer`  – TCP transmit buffer size in bytes
pub async fn http1_connect(
    reactor: &ReactorHandle,
    addr: IpAddress,
    port: u16,
    local_port: u16,
    rx_buffer: usize,
    tx_buffer: usize,
) -> Result<Connection, Error> {
    Connection::http1(reactor, addr, port, local_port, rx_buffer, tx_buffer).await
}

/// Create an HTTP/2 connection to the given address.
///
/// Convenience wrapper around [`Connection::http2`].
///
/// # Arguments
/// * `reactor`    – reactor handle for this lcore
/// * `addr`       – remote IP address
/// * `port`       – remote port
/// * `local_port` – ephemeral source port
/// * `rx_buffer`  – TCP receive buffer size in bytes
/// * `tx_buffer`  – TCP transmit buffer size in bytes
pub async fn http2_connect(
    reactor: &ReactorHandle,
    addr: IpAddress,
    port: u16,
    local_port: u16,
    rx_buffer: usize,
    tx_buffer: usize,
) -> Result<Connection, Error> {
    Connection::http2(reactor, addr, port, local_port, rx_buffer, tx_buffer).await
}

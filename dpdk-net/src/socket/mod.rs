//! Async socket implementations for TCP and UDP.
//!
//! This module provides async TCP and UDP sockets backed by DPDK and smoltcp.
//!
//! # TCP Sockets
//!
//! - [`TcpStream`]: A connected TCP stream for bidirectional data transfer
//! - [`TcpListener`]: A TCP listener for accepting incoming connections
//!
//! # UDP Sockets
//!
//! - [`UdpSocket`]: A UDP socket for connectionless datagram transfer

mod tcp;
mod udp;

pub use tcp::{AcceptFuture, TcpListener, TcpStream, WaitConnectedFuture};
pub use udp::{UdpRecvFuture, UdpSendFuture, UdpSocket};

// Re-export smoltcp error types for convenience
pub use smoltcp::socket::tcp::{ConnectError, ListenError};
pub use smoltcp::socket::udp::{
    BindError as UdpBindError, RecvError as UdpRecvError, SendError as UdpSendError, UdpMetadata,
};

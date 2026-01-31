//! Async networking with DPDK and smoltcp
//!
//! This module provides async/await support for TCP connections using DPDK
//! as the underlying packet I/O layer and smoltcp for the TCP/IP stack.
//!
//! # Architecture
//!
//! ## Runtime Abstraction
//!
//! This module is runtime-agnostic via the [`Runtime`] trait. A [`TokioRuntime`]
//! implementation is provided for tokio. To use a different runtime, implement
//! the `Runtime` trait.
//!
//! The user must:
//! 1. Create an async runtime (e.g., tokio `current_thread`)
//! 2. Spawn the reactor's `run()` method as a background task
//! 3. Use `TcpStream` and `TcpListener` normally in async code
//!
//! ## DPDK is Poll-Based
//!
//! Unlike interrupt-driven systems (tokio with epoll), DPDK requires continuous
//! polling - there are no interrupts to notify us when packets arrive.
//! The `Reactor::run()` method polls DPDK in a loop.
//!
//! ## How Wakers Work
//!
//! 1. **Reactor polls DPDK + smoltcp** continuously in a background task
//! 2. **Socket futures register wakers** with smoltcp when they would block
//! 3. **smoltcp wakes those wakers** when socket state changes during poll
//! 4. **Tokio schedules those tasks** to run
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net::tcp::{DpdkDevice, Reactor, TcpListener, TcpStream};
//! use smoltcp::iface::Interface;
//! use smoltcp::wire::IpAddress;
//! use std::sync::atomic::AtomicBool;
//! use std::sync::Arc;
//! use tokio::runtime::Builder;
//!
//! fn example(device: DpdkDevice, iface: Interface) {
//!     // Create single-threaded tokio runtime
//!     let rt = Builder::new_current_thread().enable_all().build().unwrap();
//!
//!     rt.block_on(async {
//!         // Create reactor with DPDK device and smoltcp interface
//!         let reactor = Reactor::new(device, iface);
//!         let handle = reactor.handle();
//!         let cancel = Arc::new(AtomicBool::new(false));
//!
//!         // Spawn the reactor polling task (runs forever)
//!         tokio::task::spawn_local(async move {
//!             reactor.run(cancel).await;
//!         });
//!
//!         // Create a listening socket
//!         let mut listener = TcpListener::bind(&handle, 8080, 4096, 4096)
//!             .expect("bind failed");
//!
//!         // Accept and handle connections
//!         let stream = listener.accept().await.expect("accept failed");
//!
//!         let mut buf = [0u8; 1024];
//!         let n = stream.recv(&mut buf).await.expect("recv failed");
//!         stream.send(&buf[..n]).await.expect("send failed");
//!     });
//! }
//! ```

mod reactor;
mod runtime;
mod socket;
#[cfg(feature = "tokio")]
pub mod tokio_compat;
mod udp_socket;

pub use reactor::{Reactor, ReactorHandle, ReactorInner};
pub use runtime::Runtime;
pub use socket::{
    AcceptFuture, CloseFuture, TcpListener, TcpRecvFuture, TcpSendFuture, TcpStream,
    WaitConnectedFuture,
};
#[cfg(feature = "tokio")]
pub use tokio_compat::{TokioRuntime, TokioTcpStream};
pub use udp_socket::{UdpRecvFuture, UdpSendFuture, UdpSocket};

// Re-export smoltcp error types for convenience
pub use smoltcp::socket::tcp::{ConnectError, ListenError};
pub use smoltcp::socket::udp::{
    BindError as UdpBindError, RecvError as UdpRecvError, SendError as UdpSendError, UdpMetadata,
};

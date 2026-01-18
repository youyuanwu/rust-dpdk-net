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
//! use dpdk_net::tcp::{DpdkDeviceWithPool, Reactor, TcpListener, TcpStream};
//! use smoltcp::iface::Interface;
//! use smoltcp::wire::IpAddress;
//! use tokio::runtime::Builder;
//!
//! fn example(device: DpdkDeviceWithPool, iface: Interface) {
//!     // Create single-threaded tokio runtime
//!     let rt = Builder::new_current_thread().enable_all().build().unwrap();
//!
//!     rt.block_on(async {
//!         // Create reactor with DPDK device and smoltcp interface
//!         let reactor = Reactor::new(device, iface);
//!         let handle = reactor.handle();
//!
//!         // Spawn the reactor polling task (runs forever)
//!         tokio::task::spawn_local(async move {
//!             reactor.run().await;
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

mod runtime;
mod socket;
#[cfg(feature = "tokio")]
pub mod tokio_compat;

pub use runtime::Runtime;
pub use socket::{
    AcceptFuture, CloseFuture, TcpListener, TcpRecvFuture, TcpSendFuture, TcpStream,
    WaitConnectedFuture,
};
#[cfg(feature = "tokio")]
pub use tokio_compat::{TokioRuntime, TokioTcpStream};

// Re-export smoltcp error types for convenience
pub use smoltcp::socket::tcp::{ConnectError, ListenError};

use super::DpdkDeviceWithPool;
use smoltcp::iface::{Interface, PollIngressSingleResult, SocketSet};
use smoltcp::phy::Device;
use smoltcp::time::Instant;
use std::cell::RefCell;
use std::rc::Rc;

/// Default number of packets to process before yielding to other tasks.
/// This balances responsiveness with throughput.
const DEFAULT_INGRESS_BATCH_SIZE: usize = 32;

/// Shared state for the async reactor
///
/// This holds all the smoltcp state and provides interior mutability
/// so that futures can access it.
///
/// Wakers are managed by smoltcp's socket API directly via
/// `register_recv_waker()` and `register_send_waker()`.
pub struct ReactorInner<D: Device> {
    pub device: D,
    pub iface: Interface,
    pub sockets: SocketSet<'static>,
}

impl<D: Device> ReactorInner<D> {
    /// Process one incoming packet (bounded work).
    ///
    /// Returns whether a packet was processed and whether socket state changed.
    fn poll_ingress_single(&mut self, timestamp: Instant) -> PollIngressSingleResult {
        let ReactorInner {
            device,
            iface,
            sockets,
        } = self;
        iface.poll_ingress_single(timestamp, device, sockets)
    }

    /// Transmit queued packets (bounded work).
    fn poll_egress(&mut self, timestamp: Instant) {
        let ReactorInner {
            device,
            iface,
            sockets,
        } = self;
        iface.poll_egress(timestamp, device, sockets);
    }
}

/// The async reactor that drives DPDK + smoltcp
///
/// This must be polled repeatedly to make progress on network I/O.
/// Use with tokio's single-threaded runtime (`current_thread`).
pub struct Reactor<D: Device> {
    inner: Rc<RefCell<ReactorInner<D>>>,
}

impl Reactor<DpdkDeviceWithPool> {
    /// Create a new reactor with the given DPDK device and interface
    pub fn new(device: DpdkDeviceWithPool, iface: Interface) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ReactorInner {
                device,
                iface,
                sockets: SocketSet::new(vec![]),
            })),
        }
    }

    /// Get a handle to the reactor's inner state (for creating sockets)
    pub fn handle(&self) -> ReactorHandle {
        ReactorHandle {
            inner: self.inner.clone(),
        }
    }

    /// Run the reactor forever using tokio, polling DPDK continuously with bounded work.
    ///
    /// This is a convenience method equivalent to `run_with::<TokioRuntime>()`.
    /// It should be spawned as a background task using `tokio::task::spawn_local`.
    ///
    /// To avoid DoS from packet floods, this uses `poll_ingress_single()` to process
    /// packets in batches, yielding between batches. This ensures that even under
    /// heavy load, other async tasks get a chance to run.
    ///
    /// Uses the default batch size of 32 packets. For custom batch sizes, use
    /// [`run_with_batch_size`](Self::run_with_batch_size).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use dpdk_net::tcp::{DpdkDeviceWithPool, Reactor};
    /// # use smoltcp::iface::Interface;
    /// # async fn example(device: DpdkDeviceWithPool, iface: Interface) {
    /// let reactor = Reactor::new(device, iface);
    /// let handle = reactor.handle();
    ///
    /// // Spawn reactor as background task
    /// tokio::task::spawn_local(async move {
    ///     reactor.run().await;
    /// });
    ///
    /// // Now use handle to create sockets...
    /// # }
    /// ```
    #[cfg(feature = "tokio")]
    pub async fn run(self) -> ! {
        self.run_with::<TokioRuntime>(DEFAULT_INGRESS_BATCH_SIZE)
            .await
    }

    /// Run the reactor with tokio and a custom ingress batch size.
    ///
    /// This is a convenience method equivalent to `run_with::<TokioRuntime>(batch_size)`.
    ///
    /// `batch_size` controls how many packets are processed before yielding
    /// to other tasks. Higher values increase throughput but reduce responsiveness.
    /// Lower values improve latency for other tasks but add yield overhead.
    ///
    /// Recommended values:
    /// - 16-32: Good balance for mixed workloads
    /// - 64-128: High-throughput scenarios
    /// - 1-8: When latency for other tasks is critical
    #[cfg(feature = "tokio")]
    pub async fn run_with_batch_size(self, batch_size: usize) -> ! {
        self.run_with::<TokioRuntime>(batch_size).await
    }

    /// Run the reactor with a custom async runtime.
    ///
    /// This is the most flexible run method, allowing you to use any runtime
    /// that implements the [`Runtime`] trait.
    ///
    /// # Type Parameters
    ///
    /// * `R` - The runtime implementation to use for yielding
    ///
    /// # Arguments
    ///
    /// * `batch_size` - Number of packets to process before yielding
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use dpdk_net::tcp::{DpdkDeviceWithPool, Reactor};
    /// # use dpdk_net::tcp::async_net::TokioRuntime;
    /// # use smoltcp::iface::Interface;
    /// # async fn example(device: DpdkDeviceWithPool, iface: Interface) {
    /// let reactor = Reactor::new(device, iface);
    ///
    /// // Run with explicit runtime and batch size
    /// reactor.run_with::<TokioRuntime>(64).await;
    /// # }
    /// ```
    pub async fn run_with<R: Runtime>(self, batch_size: usize) -> ! {
        loop {
            let timestamp = Instant::now();
            let mut packets_processed = 0;

            // Process ingress in batches, yielding between batches
            loop {
                let result = {
                    let mut inner = self.inner.borrow_mut();
                    inner.poll_ingress_single(timestamp)
                };

                match result {
                    PollIngressSingleResult::None => break,
                    _ => {
                        packets_processed += 1;
                        if packets_processed >= batch_size {
                            // Yield after processing a batch
                            R::yield_now().await;
                            packets_processed = 0;
                        }
                    }
                }
            }

            // Process egress (bounded work - just transmits queued packets)
            {
                let mut inner = self.inner.borrow_mut();
                inner.poll_egress(timestamp);
            }

            // Yield even when no packets, to avoid busy spinning
            R::yield_now().await;
        }
    }
}

/// Handle to the reactor for creating sockets
#[derive(Clone)]
pub struct ReactorHandle {
    pub(crate) inner: Rc<RefCell<ReactorInner<DpdkDeviceWithPool>>>,
}

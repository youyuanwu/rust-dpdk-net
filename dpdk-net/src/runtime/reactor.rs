//! Async reactor for DPDK + smoltcp networking.
//!
//! The reactor drives the network stack by continuously polling DPDK for packets
//! and processing them through smoltcp.

use super::Runtime;
#[cfg(feature = "tokio")]
use super::TokioRuntime;
use crate::device::DpdkDevice;

use smoltcp::iface::{Interface, PollIngressSingleResult, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::time::Instant;
use std::cell::{Cell, RefCell};
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
    /// Orphaned sockets that are in graceful close but no longer owned by a TcpStream.
    /// These will be cleaned up once they reach Closed or TimeWait state.
    pub(crate) orphaned_closing: Vec<SocketHandle>,
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
            ..
        } = self;
        iface.poll_ingress_single(timestamp, device, sockets)
    }

    /// Transmit queued packets (bounded work).
    fn poll_egress(&mut self, timestamp: Instant) {
        let ReactorInner {
            device,
            iface,
            sockets,
            ..
        } = self;
        iface.poll_egress(timestamp, device, sockets);
    }

    /// Clean up orphaned sockets that have completed their graceful close.
    ///
    /// Sockets in TimeWait or Closed state can be safely removed.
    fn cleanup_orphaned(&mut self) {
        use smoltcp::socket::tcp::State;

        self.orphaned_closing.retain(|&handle| {
            let socket = self.sockets.get::<smoltcp::socket::tcp::Socket>(handle);
            match socket.state() {
                State::Closed | State::TimeWait => {
                    // Socket is fully closed, remove it
                    self.sockets.remove(handle);
                    false // Remove from orphan list
                }
                _ => true, // Keep in orphan list, still closing
            }
        });
    }
}

/// The async reactor that drives DPDK + smoltcp
///
/// This must be polled repeatedly to make progress on network I/O.
/// Use with tokio's single-threaded runtime (`current_thread`).
pub struct Reactor<D: Device> {
    inner: Rc<RefCell<ReactorInner<D>>>,
}

impl Reactor<DpdkDevice> {
    /// Create a new reactor with the given DPDK device and interface
    pub fn new(device: DpdkDevice, iface: Interface) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ReactorInner {
                device,
                iface,
                sockets: SocketSet::new(vec![]),
                orphaned_closing: Vec::new(),
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
    /// # use dpdk_net::device::DpdkDevice;
    /// # use dpdk_net::runtime::Reactor;
    /// # use smoltcp::iface::Interface;
    /// # use std::cell::Cell;
    /// # use std::rc::Rc;
    /// # async fn example(device: DpdkDevice, iface: Interface) {
    /// let reactor = Reactor::new(device, iface);
    /// let handle = reactor.handle();
    /// let cancel = Rc::new(Cell::new(false));
    ///
    /// // Spawn reactor as background task
    /// tokio::task::spawn_local(async move {
    ///     reactor.run(cancel).await;
    /// });
    ///
    /// // Now use handle to create sockets...
    /// # }
    /// ```
    #[cfg(feature = "tokio")]
    pub async fn run(self, cancel: Rc<Cell<bool>>) {
        self.run_with::<TokioRuntime>(DEFAULT_INGRESS_BATCH_SIZE, cancel)
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
    pub async fn run_with_batch_size(self, batch_size: usize, cancel: Rc<Cell<bool>>) {
        self.run_with::<TokioRuntime>(batch_size, cancel).await
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
    /// * `cancel` - When set to `true`, the reactor loop will exit
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use dpdk_net::device::DpdkDevice;
    /// # use dpdk_net::runtime::{Reactor, TokioRuntime};
    /// # use smoltcp::iface::Interface;
    /// # use std::cell::Cell;
    /// # use std::rc::Rc;
    /// # async fn example(device: DpdkDevice, iface: Interface) {
    /// let reactor = Reactor::new(device, iface);
    /// let cancel = Rc::new(Cell::new(false));
    ///
    /// // Run with explicit runtime, batch size, and cancel flag
    /// reactor.run_with::<TokioRuntime>(64, cancel).await;
    /// # }
    /// ```
    pub async fn run_with<R: Runtime>(self, batch_size: usize, cancel: Rc<Cell<bool>>) {
        while !cancel.get() {
            let timestamp = Instant::now();
            let mut packets_processed = 0;

            // Process ingress in batches
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
                            // Hit batch limit - break to run egress before yielding
                            // This prevents DoS: we must send ACKs/responses, not just receive
                            break;
                        }
                    }
                }
            }

            // Process egress (bounded work - just transmits queued packets)
            {
                let mut inner = self.inner.borrow_mut();
                inner.poll_egress(timestamp);
            }

            // Clean up orphaned closing sockets that have completed their handshake
            {
                let mut inner = self.inner.borrow_mut();
                inner.cleanup_orphaned();
            }

            // Yield to let other async tasks run (accept handlers, recv futures, etc.)
            // Without this, spawned tasks would starve during idle periods
            R::yield_now().await;
        }
    }
}

/// Handle to the reactor for creating sockets
#[derive(Clone)]
pub struct ReactorHandle {
    pub(crate) inner: Rc<RefCell<ReactorInner<DpdkDevice>>>,
}

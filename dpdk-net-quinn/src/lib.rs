//! Quinn/QUIC integration for dpdk-net.
//!
//! Provides [`DpdkQuinnRuntime`] — a Quinn [`Runtime`](quinn::Runtime) backed
//! by DPDK via the OS thread bridge, and [`DpdkQuinnSocket`] — a Quinn
//! [`AsyncUdpSocket`](quinn::AsyncUdpSocket) adapter over bridge UDP sockets.

mod runtime;
mod socket;

pub use runtime::DpdkQuinnRuntime;
pub use socket::{DpdkQuinnSocket, DpdkUdpPoller};

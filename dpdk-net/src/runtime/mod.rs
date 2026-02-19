//! Async reactor and runtime support for DPDK + smoltcp networking.
//!
//! This module provides the reactor pattern implementation that continuously polls
//! DPDK for packets and processes them through smoltcp. It is runtime-agnostic
//! via the [`Runtime`] trait, with a [`TokioRuntime`] implementation provided.
//!
//! # Architecture
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
//! use dpdk_net::device::DpdkDevice;
//! use dpdk_net::runtime::{Reactor, TokioRuntime};
//! use dpdk_net::socket::TcpListener;
//! use smoltcp::iface::Interface;
//! use std::cell::Cell;
//! use std::rc::Rc;
//! use tokio::runtime::Builder;
//!
//! fn example(device: DpdkDevice, iface: Interface) {
//!     let rt = Builder::new_current_thread().enable_all().build().unwrap();
//!
//!     rt.block_on(async {
//!         let reactor = Reactor::new(device, iface);
//!         let handle = reactor.handle();
//!         let cancel = Rc::new(Cell::new(false));
//!
//!         // Spawn the reactor polling task
//!         tokio::task::spawn_local(async move {
//!             reactor.run(cancel).await;
//!         });
//!
//!         // Use handle with socket types...
//!     });
//! }
//! ```

mod reactor;
#[cfg(feature = "tokio")]
pub mod tokio_compat;
mod traits;

pub use reactor::{Reactor, ReactorHandle, ReactorInner};
#[cfg(feature = "tokio")]
pub use tokio_compat::{TokioRuntime, TokioTcpStream};
pub use traits::Runtime;

//! Async reactor support for DPDK + smoltcp networking.
//!
//! This module provides the reactor pattern implementation that continuously polls
//! DPDK for packets and processes them through smoltcp. The reactor is runtime-agnostic
//! and works with any async executor (tokio, async-std, smol, etc.).
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
//! 4. **The executor schedules those tasks** to run
//!
//! # Example
//!
//! ```ignore
//! use dpdk_net::device::DpdkDevice;
//! use dpdk_net::runtime::Reactor;
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

pub use reactor::{Reactor, ReactorHandle, ReactorInner};

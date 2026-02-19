//! DPDK Application Framework.
//!
//! This crate provides `DpdkApp`, a high-level application framework that uses
//! DPDK's native lcore threading model for optimal performance.
//!
//! # Overview
//!
//! `DpdkApp` simplifies building DPDK-based network applications by:
//! - Using EAL-managed lcores (threads created by `rte_eal_init()`)
//! - Automatically mapping lcores to queues (1:1)
//! - Setting up per-queue smoltcp network stacks
//! - Supporting both server and client applications
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         DpdkApp                                 │
//! │   ┌─────────────────────────────────────────────────────────┐   │
//! │   │  EthDev Setup │ smoltcp Interface │ Reactor per Lcore   │   │
//! │   └─────────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────────┘
//!                                │
//!                                ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       Lcore Layer                               │
//! │   Main Lcore (0)  │  Worker 1  │  Worker 2  │  Worker N         │
//! │   [runs queue 0]  │ [queue 1]  │ [queue 2]  │  [queue N]        │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use dpdk_net::api::rte::eal::EalBuilder;
//! use dpdk_net_axum::{DpdkApp, WorkerContext};
//! use dpdk_net::socket::TcpListener;
//! use smoltcp::wire::Ipv4Address;
//!
//! fn main() {
//!     let _eal = EalBuilder::new()
//!         .core_list("0-3")
//!         .allow("0000:00:04.0")
//!         .init()
//!         .expect("EAL init failed");
//!     
//!     // Run app - uses all 4 lcores, 4 queues
//!     DpdkApp::new()
//!         .eth_dev(0)
//!         .ip(Ipv4Address::new(10, 0, 0, 10))
//!         .gateway(Ipv4Address::new(10, 0, 0, 1))
//!         .run(|ctx: WorkerContext| async move {
//!             let listener = TcpListener::bind(&ctx.reactor, 8080, 4096, 4096).unwrap();
//!             // ... serve until done, then return
//!         });
//! }
//! ```

mod serve;

pub use dpdk_net_util::{DpdkApp, WorkerContext};
pub use serve::serve;

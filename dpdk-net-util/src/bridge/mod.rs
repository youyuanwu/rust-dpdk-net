//! OS Thread TCP Bridge.
//!
//! Allows non-DPDK OS threads to use TCP streams that are transparently
//! proxied through DPDK lcore workers. The bridge worker runs as a
//! `spawn_local` task on each lcore, owning the real `TcpStream` (!Send),
//! and relays data through `tokio::sync::mpsc` channels which are `Send`.
//!
//! # Usage
//!
//! ```ignore
//! use dpdk_net_util::{DpdkApp, DpdkBridge};
//! use smoltcp::wire::{IpAddress, Ipv4Address};
//!
//! // 1. Create the bridge pair before run() blocks
//! let (bridge, bridge_workers) = DpdkBridge::pair();
//!
//! // 2. Hand bridge handle to OS threads
//! let handle = bridge.clone();
//! std::thread::spawn(move || {
//!     let rt = tokio::runtime::Runtime::new().unwrap();
//!     rt.block_on(async {
//!         handle.wait_ready().await;
//!         let stream = handle.connect(IpAddress::v4(10,0,0,2), 8080).await.unwrap();
//!         // use stream with futures_io::AsyncRead + AsyncWrite
//!     });
//! });
//!
//! // 3. Pass workers into run() closure
//! DpdkApp::new()
//!     .ip(Ipv4Address::new(10, 0, 0, 10))
//!     .gateway(Ipv4Address::new(10, 0, 0, 1))
//!     .run(move |ctx| {
//!         let workers = bridge_workers.clone();
//!         async move {
//!             workers.spawn(&ctx.reactor);
//!             // ... normal server code
//!         }
//!     });
//! ```

mod command;
pub mod error;
mod handle;
mod listener;
mod stream;
mod udp;
mod worker;

pub use error::BridgeError;
pub use handle::DpdkBridge;
pub use listener::BridgeTcpListener;
pub use stream::BridgeTcpStream;
pub use udp::BridgeUdpSocket;
pub use worker::BridgeWorkers;

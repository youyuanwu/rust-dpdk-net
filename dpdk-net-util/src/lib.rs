//! HTTP client for DPDK networks.
//!
//! Provides HTTP/1.1 and HTTP/2 client functionality over `dpdk-net` TCP
//! streams using hyper's low-level connection API.
//!
//! All types in this crate are `!Send` because the underlying DPDK streams
//! use `Rc<RefCell<...>>`. Use `spawn_local` / `LocalSet` for async tasks.
//!
//! # Quick start
//!
//! ```ignore
//! use dpdk_net_util::{DpdkHttpClient, Connection};
//! use dpdk_net::runtime::ReactorHandle;
//! use smoltcp::wire::IpAddress;
//!
//! async fn run(reactor: &ReactorHandle) {
//!     // Option A: helper function
//!     let mut conn = dpdk_net_util::http1_connect(
//!         reactor,
//!         IpAddress::v4(10, 0, 0, 1), 8080, 1234,
//!         16384, 16384,
//!     ).await.unwrap();
//!     // conn.send_request(req).await ...
//!
//!     // Option B: client with config
//!     let client = DpdkHttpClient::new(reactor.clone());
//!     let mut conn = client.connect(
//!         IpAddress::v4(10, 0, 0, 1), 8080, 1234
//!     ).await.unwrap();
//! }
//! ```

pub mod app;
pub mod client;
pub mod connect;
pub mod connection;
pub mod context;
pub mod error;
pub mod executor;
pub mod pool;

pub use app::DpdkApp;
pub use client::{ClientConfig, DpdkHttpClient};
pub use connect::{http1_connect, http2_connect};
pub use connection::{Connection, HttpVersion, ResponseFuture};
pub use context::WorkerContext;
pub use error::Error;
pub use executor::LocalExecutor;
pub use pool::ConnectionPool;

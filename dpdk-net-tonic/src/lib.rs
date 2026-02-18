//! gRPC support for DPDK networks via [tonic](https://docs.rs/tonic).
//!
//! This crate provides two things:
//!
//! - [`serve`] — a thin server wrapper that accepts tonic's `Router` and
//!   delegates to [`dpdk_net_axum::serve`] after converting to an axum `Router`
//!   via `.into_router()`.
//!
//! - [`DpdkGrpcChannel`] — a `!Send` gRPC channel backed by a persistent
//!   HTTP/2 connection over `dpdk-net`. Use this instead of
//!   `tonic::transport::Channel` (which requires `Send`).

mod channel;
mod serve;

pub use channel::DpdkGrpcChannel;
pub use serve::serve;

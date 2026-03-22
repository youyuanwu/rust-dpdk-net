//! Tonic gRPC integration for dpdk-net.
//!
//! Provides:
//! - [`serve`] — gRPC server wrapper that accepts tonic `Routes`
//! - [`DpdkGrpcChannel`] — `!Send` gRPC client channel over HTTP/2
//! - [`bridge`] — OS thread adapters for tonic's native transport APIs

pub mod bridge;
mod channel;
mod serve;

pub use channel::DpdkGrpcChannel;
pub use serve::serve;

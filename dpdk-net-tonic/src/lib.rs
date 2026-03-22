//! Axum and tonic gRPC integration for dpdk-net.
//!
//! Provides:
//! - [`axum::serve`] — Serve an axum `Router` on a dpdk-net `TcpListener`
//! - [`tonic::serve`] — Serve tonic gRPC `Routes` on a dpdk-net `TcpListener`
//! - [`tonic::DpdkGrpcChannel`] — `!Send` gRPC client channel over HTTP/2
//! - [`tonic::bridge`] — OS thread adapters for tonic's native transport APIs

pub mod axum;
pub mod tonic;

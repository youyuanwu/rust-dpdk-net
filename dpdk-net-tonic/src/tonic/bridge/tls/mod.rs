//! TLS support for bridge tonic gRPC via [`tonic_tls`] + OpenSSL.
//!
//! Provides:
//! - [`BridgeTransport`] — `tonic_tls::Transport` impl for TLS client connections
//! - `tonic_tls::Incoming` impl for [`BridgeIncoming`](super::BridgeIncoming)
//!
//! Use `tonic_tls::openssl::TlsConnector` (client) and
//! `tonic_tls::openssl::TlsIncoming` (server) with these types.

mod incoming;
mod transport;

pub use transport::BridgeTransport;

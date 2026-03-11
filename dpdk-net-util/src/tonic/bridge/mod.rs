//! OS Thread Bridge adapters for tonic gRPC.
//!
//! Provides types that adapt [`DpdkBridge`](crate::DpdkBridge) streams
//! for use with tonic's native `transport` APIs:
//!
//! - [`BridgeIo`] — IO adapter with [`Connected`](tonic::transport::server::Connected) trait
//! - [`BridgeConnector`] — `tower::Service<Uri>` for `Endpoint::connect_with_connector()`
//! - [`BridgeIncoming`] — `Stream` adapter for `Server::serve_with_incoming_shutdown()`

mod connector;
mod incoming;
mod io;

pub use connector::BridgeConnector;
pub use incoming::BridgeIncoming;
pub use io::BridgeIo;

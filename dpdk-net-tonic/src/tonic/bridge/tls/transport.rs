//! [`BridgeTransport`] — `tonic_tls::Transport` for TLS client connections
//! over [`DpdkBridge`](dpdk_net_util::DpdkBridge).

use dpdk_net_util::{BridgeError, DpdkBridge};
use http::Uri;
use smoltcp::wire::IpAddress;

use super::super::io::BridgeIo;

/// Transport that produces [`BridgeIo`] connections via [`DpdkBridge`](dpdk_net_util::DpdkBridge).
///
/// Implements [`tonic_tls::Transport`] so it can be passed to
/// `tonic_tls::openssl::TlsConnector::new(transport, ssl_connector, domain)`.
///
/// Unlike [`BridgeConnector`](super::super::BridgeConnector) which implements
/// `tower::Service<Uri>` directly, this type fits tonic-tls's transport
/// abstraction — tonic-tls adds TLS wrapping and `TokioIo` internally.
#[derive(Clone)]
pub struct BridgeTransport {
    bridge: DpdkBridge,
}

impl BridgeTransport {
    /// Create a new transport backed by the given bridge.
    pub fn new(bridge: DpdkBridge) -> Self {
        Self { bridge }
    }
}

impl tonic_tls::Transport for BridgeTransport {
    type Io = BridgeIo;
    type Error = BridgeError;

    async fn connect(&self, uri: &Uri) -> Result<BridgeIo, BridgeError> {
        let host = uri.host().expect("URI must have a host");
        let addr: IpAddress = host.parse().expect("URI host must be an IP address");
        let port = uri.port_u16().expect("URI must have a port");
        let stream = self.bridge.connect(addr, port).await?;
        Ok(BridgeIo::new(stream))
    }
}

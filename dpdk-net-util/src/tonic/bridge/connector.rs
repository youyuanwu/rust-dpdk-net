//! [`BridgeConnector`] — tonic client connector over [`DpdkBridge`].
//!
//! Implements `tower::Service<Uri>` so it can be passed to
//! `tonic::transport::Endpoint::connect_with_connector()`.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::Uri;
use smoltcp::wire::IpAddress;

use hyper_util::rt::TokioIo;

use crate::{BridgeError, DpdkBridge};

use super::io::BridgeIo;

/// Connector that establishes TCP connections via [`DpdkBridge`].
///
/// Pass to [`tonic::transport::Endpoint::connect_with_connector()`]
/// to get a standard `tonic::transport::Channel` backed by DPDK.
///
/// Returns `TokioIo<BridgeIo>` because tonic's connector path requires
/// `hyper::rt::io::Read + Write` (hyper 1.x IO traits), which `TokioIo`
/// provides for any `tokio::io::AsyncRead + AsyncWrite` type.
///
/// `Clone` because `DpdkBridge` is `Clone`.
#[derive(Clone)]
pub struct BridgeConnector {
    bridge: DpdkBridge,
}

impl BridgeConnector {
    /// Create a new connector backed by the given bridge.
    pub fn new(bridge: DpdkBridge) -> Self {
        Self { bridge }
    }
}

impl tower::Service<Uri> for BridgeConnector {
    type Response = TokioIo<BridgeIo>;
    type Error = BridgeError;
    type Future = Pin<Box<dyn Future<Output = Result<TokioIo<BridgeIo>, BridgeError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let bridge = self.bridge.clone();
        Box::pin(async move {
            let host = uri.host().expect("URI must have a host");
            let addr: IpAddress = host.parse().expect("URI host must be an IP address");
            let port = uri.port_u16().expect("URI must have a port");
            let stream = bridge.connect(addr, port).await?;
            Ok(TokioIo::new(BridgeIo::new(stream)))
        })
    }
}

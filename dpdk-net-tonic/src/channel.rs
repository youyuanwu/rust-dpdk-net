//! `!Send` gRPC channel backed by a persistent HTTP/2 connection.
//!
//! [`DpdkGrpcChannel`] wraps [`dpdk_net_hyper::Connection`] (HTTP/2 only) and
//! implements `tower::Service<Request<tonic::body::Body>>`, satisfying tonic's
//! [`GrpcService`](tonic::client::GrpcService) trait via blanket impl.
//!
//! Use this instead of `tonic::transport::Channel`, which requires `Send`.

use std::task::{Context, Poll};

use dpdk_net::runtime::ReactorHandle;
use dpdk_net_hyper::{Connection, ResponseFuture};
use http::Uri;
use http::uri::{Authority, Scheme};

/// A `!Send` gRPC channel backed by a persistent HTTP/2 connection
/// over dpdk-net transport.
///
/// Implements `tower::Service<Request<tonic::body::Body>>`, which satisfies
/// `tonic::client::GrpcService` via blanket impl.
///
/// Not `Clone` — create one channel per tonic client instance.
pub struct DpdkGrpcChannel {
    conn: Connection,
    scheme: Scheme,
    authority: Authority,
}

impl DpdkGrpcChannel {
    /// Connect to a gRPC server over dpdk-net.
    ///
    /// The URI must have scheme (`http`), host (an IP address), and port.
    /// Example: `http://192.168.1.1:50051`
    ///
    /// Establishes a TCP connection and completes the HTTP/2 handshake
    /// using [`LocalExecutor`](dpdk_net_hyper::LocalExecutor) (no `Send`
    /// required).
    ///
    /// Uses an ephemeral local port (`0`) and default buffer sizes
    /// (4096 bytes rx/tx).
    pub async fn connect(reactor: &ReactorHandle, uri: Uri) -> Result<Self, dpdk_net_hyper::Error> {
        Self::connect_with(reactor, uri, 0, 4096, 4096).await
    }

    /// Connect with explicit local port and buffer sizes.
    ///
    /// See [`connect`](Self::connect) for URI format requirements.
    pub async fn connect_with(
        reactor: &ReactorHandle,
        uri: Uri,
        local_port: u16,
        rx_buffer: usize,
        tx_buffer: usize,
    ) -> Result<Self, dpdk_net_hyper::Error> {
        let scheme = uri.scheme().expect("URI must have a scheme").clone();
        let authority = uri.authority().expect("URI must have authority").clone();
        let addr: smoltcp::wire::IpAddress = authority
            .host()
            .parse()
            .expect("URI host must be an IP address");
        let port = uri.port_u16().expect("URI must have a port");
        let conn = Connection::http2(reactor, addr, port, local_port, rx_buffer, tx_buffer).await?;
        Ok(Self {
            conn,
            scheme,
            authority,
        })
    }

    /// Check if the underlying HTTP/2 connection is still usable.
    pub fn is_ready(&self) -> bool {
        self.conn.is_ready()
    }
}

impl tower::Service<http::Request<tonic::body::Body>> for DpdkGrpcChannel {
    type Response = http::Response<hyper::body::Incoming>;
    type Error = dpdk_net_hyper::Error;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.conn.is_ready() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(dpdk_net_hyper::Error::ConnectionNotReady))
        }
    }

    fn call(&mut self, mut req: http::Request<tonic::body::Body>) -> Self::Future {
        // Tonic generates requests with only a path (no scheme/authority).
        // HTTP/2 over hyper requires a full URI, so inject them here.
        if req.uri().scheme().is_none() {
            let path = req
                .uri()
                .path_and_query()
                .cloned()
                .unwrap_or_else(|| http::uri::PathAndQuery::from_static("/"));
            let uri = http::Uri::builder()
                .scheme(self.scheme.clone())
                .authority(self.authority.clone())
                .path_and_query(path)
                .build()
                .expect("valid URI");
            *req.uri_mut() = uri;
        }
        self.conn.send_request(req)
    }
}

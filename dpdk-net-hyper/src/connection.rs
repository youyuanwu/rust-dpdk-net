use std::pin::Pin;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::client::conn::{http1, http2};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;

use dpdk_net::runtime::ReactorHandle;
use dpdk_net::runtime::tokio_compat::TokioTcpStream;
use dpdk_net::socket::TcpStream;
use smoltcp::wire::IpAddress;

use crate::error::Error;
use crate::executor::LocalExecutor;

/// HTTP version to use for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpVersion {
    Http1,
    Http2,
}

/// A persistent HTTP connection over a DPDK TCP stream.
///
/// Wraps hyper's low-level `SendRequest` handle. Each connection holds
/// an open TCP stream and can be used for multiple requests.
///
/// # Note
/// This type is `!Send` because the underlying DPDK TCP stream uses `Rc`.
/// All usage must be on a single lcore via `spawn_local`.
pub struct Connection {
    sender: ConnectionSender,
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A body type that does not require `Send` or `Sync`.
///
/// Uses `Pin<Box<dyn Body>>` since `http_body_util::{BoxBody, UnsyncBoxBody}`
/// both require `Send` on construction, which dpdk-net streams cannot provide.
type BoxBody = Pin<Box<dyn hyper::body::Body<Data = Bytes, Error = BoxError>>>;

enum ConnectionSender {
    Http1(http1::SendRequest<BoxBody>),
    Http2(http2::SendRequest<BoxBody>),
}

/// Convert any compatible body into our internal `BoxBody`.
fn into_box_body<B>(body: B) -> BoxBody
where
    B: hyper::body::Body<Data = Bytes> + 'static,
    B::Error: Into<BoxError>,
{
    Box::pin(body.map_err(|e| -> BoxError { e.into() }))
}

impl Connection {
    /// Create a new HTTP/1.1 connection.
    ///
    /// The connection driver future is spawned onto the local task set
    /// via `spawn_local`.
    pub async fn http1(
        reactor: &ReactorHandle,
        addr: IpAddress,
        port: u16,
        local_port: u16,
        rx_buffer: usize,
        tx_buffer: usize,
    ) -> Result<Self, Error> {
        let io = Self::connect_tcp(reactor, addr, port, local_port, rx_buffer, tx_buffer).await?;
        let (sender, conn) = http1::handshake(io).await.map_err(Error::Handshake)?;
        tokio::task::spawn_local(async move {
            if let Err(e) = conn.await {
                tracing::error!(error = ?e, "HTTP/1.1 connection error");
            }
        });
        Ok(Self {
            sender: ConnectionSender::Http1(sender),
        })
    }

    /// Create a new HTTP/2 connection.
    ///
    /// Uses [`LocalExecutor`] for hyper's background tasks since the
    /// stream is `!Send`.
    pub async fn http2(
        reactor: &ReactorHandle,
        addr: IpAddress,
        port: u16,
        local_port: u16,
        rx_buffer: usize,
        tx_buffer: usize,
    ) -> Result<Self, Error> {
        let io = Self::connect_tcp(reactor, addr, port, local_port, rx_buffer, tx_buffer).await?;
        let (sender, conn) = http2::handshake(LocalExecutor, io)
            .await
            .map_err(Error::Handshake)?;
        tokio::task::spawn_local(async move {
            if let Err(e) = conn.await {
                tracing::error!(error = ?e, "HTTP/2 connection error");
            }
        });
        Ok(Self {
            sender: ConnectionSender::Http2(sender),
        })
    }

    /// Send a request over this connection.
    pub async fn send_request<B>(
        &mut self,
        request: Request<B>,
    ) -> Result<Response<Incoming>, Error>
    where
        B: hyper::body::Body<Data = Bytes> + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let request = request.map(into_box_body);
        match &mut self.sender {
            ConnectionSender::Http1(sender) => {
                sender.send_request(request).await.map_err(Error::Request)
            }
            ConnectionSender::Http2(sender) => {
                sender.send_request(request).await.map_err(Error::Request)
            }
        }
    }

    /// Check if the connection is still usable for sending requests.
    pub fn is_ready(&self) -> bool {
        match &self.sender {
            ConnectionSender::Http1(s) => s.is_ready(),
            ConnectionSender::Http2(s) => s.is_ready(),
        }
    }

    /// Returns the HTTP version of this connection.
    pub fn version(&self) -> HttpVersion {
        match &self.sender {
            ConnectionSender::Http1(_) => HttpVersion::Http1,
            ConnectionSender::Http2(_) => HttpVersion::Http2,
        }
    }

    /// Establish a DPDK TCP connection and wrap it for hyper.
    async fn connect_tcp(
        reactor: &ReactorHandle,
        addr: IpAddress,
        port: u16,
        local_port: u16,
        rx_buffer: usize,
        tx_buffer: usize,
    ) -> Result<TokioIo<TokioTcpStream>, Error> {
        let stream = TcpStream::connect(reactor, addr, port, local_port, rx_buffer, tx_buffer)?;
        stream
            .wait_connected()
            .await
            .map_err(|()| Error::ConnectionFailed)?;
        Ok(TokioIo::new(TokioTcpStream::new(stream)))
    }
}

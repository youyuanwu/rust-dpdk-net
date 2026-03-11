//! [`BridgeIo`] — tonic-compatible IO adapter for [`BridgeTcpStream`].
//!
//! Wraps `Compat<BridgeTcpStream>` to add the [`Connected`] trait
//! required by tonic's `serve_with_incoming_shutdown` and
//! `connect_with_connector`.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};
use tonic::transport::server::Connected;

use crate::BridgeTcpStream;

/// Bridge TCP stream adapted for tonic transport.
///
/// Implements `tokio::io::AsyncRead + AsyncWrite + Connected + Unpin + Send`.
pub struct BridgeIo {
    inner: Compat<BridgeTcpStream>,
}

impl BridgeIo {
    /// Wrap a [`BridgeTcpStream`] for use with tonic transport.
    pub fn new(stream: BridgeTcpStream) -> Self {
        Self {
            inner: stream.compat(),
        }
    }
}

impl Connected for BridgeIo {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl tokio::io::AsyncRead for BridgeIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for BridgeIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

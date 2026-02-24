//! Tokio compatibility wrappers for async sockets.
//!
//! This module provides:
//! - [`TokioRuntime`]: Implementation of the [`Runtime`](super::Runtime) trait for tokio
//! - [`TokioTcpStream`]: A wrapper around [`TcpStream`](crate::socket::TcpStream) that implements
//!   tokio's [`AsyncRead`](tokio::io::AsyncRead) and [`AsyncWrite`](tokio::io::AsyncWrite) traits
//! - [`TokioUdpSocket`]: A wrapper around [`UdpSocket`](crate::socket::UdpSocket) providing
//!   tokio-style async methods for datagram I/O
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net::socket::TcpStream;
//! use dpdk_net::runtime::tokio_compat::TokioTcpStream;
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//!
//! async fn example(stream: TcpStream) {
//!     let mut stream = TokioTcpStream::new(stream);
//!
//!     // Use tokio's AsyncRead/AsyncWrite traits
//!     let mut buf = [0u8; 1024];
//!     let n = stream.read(&mut buf).await.unwrap();
//!     stream.write_all(&buf[..n]).await.unwrap();
//! }
//! ```

use super::Runtime;
use crate::socket::{TcpStream, UdpSocket};
use smoltcp::socket::tcp::{self, RecvError, State};
use smoltcp::socket::udp::{RecvError as UdpRecvError, SendError as UdpSendError, UdpMetadata};
use smoltcp::wire::IpEndpoint;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Tokio runtime implementation.
///
/// This is the default runtime for use with tokio's single-threaded executor.
/// Use with [`Reactor::run`](super::Reactor::run) or [`Reactor::run_with`](super::Reactor::run_with).
pub struct TokioRuntime;

impl Runtime for TokioRuntime {
    fn yield_now() -> impl Future<Output = ()> {
        tokio::task::yield_now()
    }
}

/// A wrapper around [`TcpStream`] that implements tokio's async I/O traits.
///
/// This allows using the DPDK-backed TCP stream with tokio's ecosystem,
/// including utilities like [`AsyncReadExt`](tokio::io::AsyncReadExt),
/// [`AsyncWriteExt`](tokio::io::AsyncWriteExt), and codec frameworks.
pub struct TokioTcpStream {
    inner: TcpStream,
}

impl TokioTcpStream {
    /// Create a new tokio-compatible wrapper around a [`TcpStream`].
    pub fn new(stream: TcpStream) -> Self {
        Self { inner: stream }
    }

    /// Get a reference to the underlying [`TcpStream`].
    pub fn get_ref(&self) -> &TcpStream {
        &self.inner
    }

    /// Get a mutable reference to the underlying [`TcpStream`].
    pub fn get_mut(&mut self) -> &mut TcpStream {
        &mut self.inner
    }

    /// Consume this wrapper and return the underlying [`TcpStream`].
    pub fn into_inner(self) -> TcpStream {
        self.inner
    }
}

impl AsyncRead for TokioTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let mut inner = this.inner.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(this.inner.handle);

        // Try to receive data into the unfilled portion of the buffer
        let unfilled = buf.initialize_unfilled();
        if unfilled.is_empty() {
            return Poll::Ready(Ok(()));
        }

        match socket.recv_slice(unfilled) {
            Ok(0) => {
                // No data available yet - register waker and wait
                socket.register_recv_waker(cx.waker());
                Poll::Pending
            }
            Ok(n) => {
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            Err(RecvError::Finished) => {
                // EOF - connection closed gracefully
                Poll::Ready(Ok(()))
            }
            Err(RecvError::InvalidState) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "socket in invalid state for receiving",
            ))),
        }
    }
}

impl AsyncWrite for TokioTcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let mut inner = this.inner.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(this.inner.handle);

        match socket.send_slice(buf) {
            Ok(0) if !buf.is_empty() => {
                // No space in send buffer - register waker and wait
                socket.register_send_waker(cx.waker());
                Poll::Pending
            }
            Ok(n) => Poll::Ready(Ok(n)),
            Err(smoltcp::socket::tcp::SendError::InvalidState) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "socket in invalid state for sending",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let mut inner = this.inner.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(this.inner.handle);

        // smoltcp doesn't have explicit flush - data is sent when egress is polled.
        // We consider flush complete when the send buffer is empty.
        if socket.send_queue() == 0 {
            Poll::Ready(Ok(()))
        } else {
            socket.register_send_waker(cx.waker());
            Poll::Pending
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        // First, initiate close if not already closing
        {
            let mut inner = this.inner.reactor.borrow_mut();
            let socket = inner.sockets.get_mut::<tcp::Socket>(this.inner.handle);

            match socket.state() {
                // Already fully closed
                State::Closed | State::TimeWait => return Poll::Ready(Ok(())),
                // Already initiated close, wait for completion
                State::FinWait1 | State::FinWait2 | State::Closing | State::LastAck => {}
                // Need to initiate close
                _ => socket.close(),
            }
        }

        // Wait for close to complete
        {
            let mut inner = this.inner.reactor.borrow_mut();
            let socket = inner.sockets.get_mut::<tcp::Socket>(this.inner.handle);

            match socket.state() {
                State::Closed | State::TimeWait => Poll::Ready(Ok(())),
                _ => {
                    socket.register_recv_waker(cx.waker());
                    Poll::Pending
                }
            }
        }
    }
}

impl From<TcpStream> for TokioTcpStream {
    fn from(stream: TcpStream) -> Self {
        Self::new(stream)
    }
}

/// A wrapper around [`UdpSocket`] providing tokio-style async methods.
///
/// Unlike [`TokioTcpStream`] which implements `AsyncRead`/`AsyncWrite` (stream
/// protocols), UDP is datagram-based. This wrapper provides `send_to()` and
/// `recv_from()` async methods that match tokio's `UdpSocket` API.
///
/// # Example
///
/// ```no_run
/// use dpdk_net::socket::UdpSocket;
/// use dpdk_net::runtime::tokio_compat::TokioUdpSocket;
/// use smoltcp::wire::IpEndpoint;
///
/// async fn example(socket: UdpSocket) {
///     let socket = TokioUdpSocket::new(socket);
///     let mut buf = [0u8; 1500];
///     let (n, meta) = socket.recv_from(&mut buf).await.unwrap();
///     socket.send_to(&buf[..n], meta.endpoint).await.unwrap();
/// }
/// ```
pub struct TokioUdpSocket {
    inner: UdpSocket,
}

impl TokioUdpSocket {
    /// Create a new tokio-compatible wrapper around a [`UdpSocket`].
    pub fn new(socket: UdpSocket) -> Self {
        Self { inner: socket }
    }

    /// Get a reference to the underlying [`UdpSocket`].
    pub fn get_ref(&self) -> &UdpSocket {
        &self.inner
    }

    /// Get a mutable reference to the underlying [`UdpSocket`].
    pub fn get_mut(&mut self) -> &mut UdpSocket {
        &mut self.inner
    }

    /// Consume this wrapper and return the underlying [`UdpSocket`].
    pub fn into_inner(self) -> UdpSocket {
        self.inner
    }

    /// Set the default remote endpoint (connected mode).
    pub fn connect(&mut self, endpoint: IpEndpoint) {
        self.inner.connect(endpoint);
    }

    /// Send a datagram to the specified endpoint.
    pub async fn send_to(&self, data: &[u8], endpoint: IpEndpoint) -> Result<usize, UdpSendError> {
        self.inner.send_to(data, endpoint).await
    }

    /// Send a datagram to the connected endpoint.
    ///
    /// # Panics
    /// Panics if [`connect`](Self::connect) was not called first.
    pub async fn send(&self, data: &[u8]) -> Result<usize, UdpSendError> {
        self.inner.send(data).await
    }

    /// Receive a datagram, returning bytes read and source metadata.
    pub async fn recv_from(
        &self,
        buf: &mut [u8],
    ) -> Result<(usize, UdpMetadata), UdpRecvError> {
        self.inner.recv_from(buf).await
    }

    /// Receive a datagram, returning only the number of bytes read.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize, UdpRecvError> {
        self.inner.recv(buf).await
    }
}

impl From<UdpSocket> for TokioUdpSocket {
    fn from(socket: UdpSocket) -> Self {
        Self::new(socket)
    }
}

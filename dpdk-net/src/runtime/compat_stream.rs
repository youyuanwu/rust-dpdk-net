//! Tokio compatibility wrappers for async TCP sockets.
//!
//! This module provides:
//! - [`TokioRuntime`]: Implementation of the [`Runtime`](super::Runtime) trait for tokio
//! - [`TokioTcpStream`]: A wrapper around [`TcpStream`](crate::socket::TcpStream) that implements
//!   [`futures_io::AsyncRead`] and [`futures_io::AsyncWrite`] traits
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net::socket::TcpStream;
//! use dpdk_net::runtime::compat_stream::AsyncTcpStream;
//! use futures_io::{AsyncRead, AsyncWrite};
//!
//! async fn example(stream: TcpStream) {
//!     let stream = AsyncTcpStream::new(stream);
//!     // stream implements futures_io::AsyncRead + AsyncWrite
//! }
//! ```

use super::Runtime;
use crate::socket::TcpStream;
use futures_io::{AsyncRead, AsyncWrite};
use smoltcp::socket::tcp::{self, RecvError, State};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

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

/// A wrapper around [`TcpStream`] that implements [`futures_io::AsyncRead`] and [`futures_io::AsyncWrite`].
///
/// This allows using the DPDK-backed TCP stream with async ecosystems.
/// Use [`tokio_util::compat::FuturesAsyncReadCompatExt`] to adapt for tokio-based
/// consumers like hyper's [`TokioIo`](hyper_util::rt::TokioIo).
pub struct AsyncTcpStream {
    inner: TcpStream,
}

impl AsyncTcpStream {
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

impl AsyncRead for AsyncTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let mut inner = this.inner.reactor.borrow_mut();
        let socket = inner.sockets.get_mut::<tcp::Socket>(this.inner.handle);

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        match socket.recv_slice(buf) {
            Ok(0) => {
                // No data available yet - register waker and wait
                socket.register_recv_waker(cx.waker());
                Poll::Pending
            }
            Ok(n) => Poll::Ready(Ok(n)),
            Err(RecvError::Finished) => {
                // EOF - connection closed gracefully
                Poll::Ready(Ok(0))
            }
            Err(RecvError::InvalidState) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "socket in invalid state for receiving",
            ))),
        }
    }
}

impl AsyncWrite for AsyncTcpStream {
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
        let inner = this.inner.reactor.borrow_mut();
        let socket = inner.sockets.get::<tcp::Socket>(this.inner.handle);

        // smoltcp doesn't have explicit flush - data is sent when egress is polled.
        // We consider flush complete when the send buffer is empty.
        if socket.send_queue() == 0 {
            Poll::Ready(Ok(()))
        } else {
            // Register waker to be notified when send buffer drains
            drop(inner);
            let mut inner = this.inner.reactor.borrow_mut();
            let socket = inner.sockets.get_mut::<tcp::Socket>(this.inner.handle);
            socket.register_send_waker(cx.waker());
            Poll::Pending
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
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
                    socket.register_send_waker(cx.waker());
                    Poll::Pending
                }
            }
        }
    }
}

impl From<TcpStream> for AsyncTcpStream {
    fn from(stream: TcpStream) -> Self {
        Self::new(stream)
    }
}

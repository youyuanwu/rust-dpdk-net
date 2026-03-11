use bytes::{Buf, Bytes};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::PollSender;

use super::error::BridgeError;

/// A TCP stream proxied through DPDK. `Send`, `!Sync`.
///
/// Data written here is forwarded to the DPDK `TcpStream` on an lcore.
/// Data received by the DPDK `TcpStream` is forwarded back here.
///
/// Implements `futures_io::AsyncRead + AsyncWrite`.
pub struct BridgeTcpStream {
    /// Send data to the lcore bridge worker for transmission.
    data_tx: PollSender<Bytes>,
    /// Receive data from the lcore bridge worker (ingress from NIC).
    data_rx: mpsc::Receiver<Result<Bytes, BridgeError>>,
    /// Buffered partial chunk from the previous recv that didn't fit
    /// into the caller's read buffer.
    read_buf: Bytes,
    /// Signals shutdown/close to the relay task.
    close_tx: Option<oneshot::Sender<()>>,
}

impl BridgeTcpStream {
    pub(crate) fn new(
        data_tx: mpsc::Sender<Bytes>,
        data_rx: mpsc::Receiver<Result<Bytes, BridgeError>>,
        close_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            data_tx: PollSender::new(data_tx),
            data_rx,
            read_buf: Bytes::new(),
            close_tx: Some(close_tx),
        }
    }
}

impl futures_io::AsyncRead for BridgeTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        // 1. Drain leftover bytes from previous recv
        if !this.read_buf.is_empty() {
            let n = std::cmp::min(buf.len(), this.read_buf.len());
            buf[..n].copy_from_slice(&this.read_buf[..n]);
            this.read_buf.advance(n);
            return Poll::Ready(Ok(n));
        }

        // 2. Poll the channel for the next chunk
        match this.data_rx.poll_recv(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                if chunk.is_empty() {
                    // EOF — lcore side closed
                    return Poll::Ready(Ok(0));
                }
                let n = std::cmp::min(buf.len(), chunk.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                if n < chunk.len() {
                    this.read_buf = chunk.slice(n..);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(e.into())),
            Poll::Ready(None) => Poll::Ready(Ok(0)), // channel closed = EOF
            Poll::Pending => Poll::Pending,
        }
    }
}

impl futures_io::AsyncWrite for BridgeTcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        match this.data_tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let data = Bytes::copy_from_slice(buf);
                let len = data.len();
                this.data_tx
                    .send_item(data)
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "lcore gone"))?;
                Poll::Ready(Ok(len))
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "lcore relay task exited",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // The relay task writes eagerly; no local buffering to flush.
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        // Signal the relay task to close the DPDK stream
        if let Some(tx) = this.close_tx.take() {
            let _ = tx.send(());
        }
        Poll::Ready(Ok(()))
    }
}

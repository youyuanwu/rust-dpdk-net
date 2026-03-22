use std::io;
use std::io::IoSliceMut;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use quinn::udp::RecvMeta;
use quinn::{AsyncUdpSocket, UdpPoller};
use tokio::io::ReadBuf;

use dpdk_net_util::bridge::BridgeUdpSocket;

/// Quinn [`AsyncUdpSocket`] adapter over a [`BridgeUdpSocket`].
///
/// Translates between Quinn's [`quinn::Transmit`]/[`RecvMeta`] types and the
/// bridge's `send_to`/`recv_from` API. One datagram per call (no GSO/GRO).
#[derive(Debug)]
pub struct DpdkQuinnSocket {
    inner: Arc<BridgeUdpSocket>,
}

impl DpdkQuinnSocket {
    pub fn new(socket: BridgeUdpSocket) -> Self {
        Self {
            inner: Arc::new(socket),
        }
    }
}

impl AsyncUdpSocket for DpdkQuinnSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(DpdkUdpPoller { socket: self })
    }

    fn try_send(&self, transmit: &quinn::udp::Transmit<'_>) -> io::Result<()> {
        self.inner
            .try_send_to(transmit.contents, transmit.destination)?;
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let mut read_buf = ReadBuf::new(&mut bufs[0]);
        match self.inner.poll_recv_from(cx, &mut read_buf) {
            Poll::Ready(Ok(addr)) => {
                let len = read_buf.filled().len();
                meta[0] = RecvMeta {
                    addr,
                    len,
                    stride: len,
                    ecn: None,
                    dst_ip: self.inner.local_addr().ok().map(|a| a.ip()),
                };
                Poll::Ready(Ok(1))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn max_transmit_segments(&self) -> usize {
        1 // no GSO
    }

    fn max_receive_segments(&self) -> usize {
        1 // no GRO
    }

    fn may_fragment(&self) -> bool {
        false // no kernel IP stack; QUIC handles its own path MTU discovery
    }
}

/// Poller returned by [`DpdkQuinnSocket::create_io_poller`].
///
/// Quinn calls [`poll_writable`](UdpPoller::poll_writable) to know when
/// [`try_send`](AsyncUdpSocket::try_send) should be retried after
/// returning `WouldBlock`.
#[derive(Debug)]
pub struct DpdkUdpPoller {
    socket: Arc<DpdkQuinnSocket>,
}

impl UdpPoller for DpdkUdpPoller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.socket.inner.poll_send_ready(cx)
    }
}

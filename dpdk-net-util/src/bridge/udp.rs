use bytes::Bytes;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::task::{Context, Poll};

use tokio::io::ReadBuf;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::PollSender;

/// Datagram in flight between OS thread and lcore relay.
pub(crate) struct UdpDatagram {
    pub payload: Bytes,
    pub addr: SocketAddr,
}

/// A UDP socket proxied through DPDK. `Send + Sync`.
///
/// Mirrors `tokio::net::UdpSocket` API — all methods take `&self` so it can
/// be shared via `Arc`. Internally holds channel handles to the lcore's
/// `udp_relay_task`.
///
/// Created via [`DpdkBridge::bind_udp()`](super::handle::DpdkBridge::bind_udp).
pub struct BridgeUdpSocket {
    /// Send datagrams to the lcore relay (OS → lcore).
    tx: Mutex<PollSender<UdpDatagram>>,
    /// Receive datagrams from the lcore relay (lcore → OS).
    rx: Mutex<RxState>,
    /// Cached local address.
    local_addr: SocketAddr,
    /// Connected peer address, if any.
    peer_addr: Mutex<Option<SocketAddr>>,
}

/// Receiver state with one-slot peek buffer.
struct RxState {
    receiver: mpsc::Receiver<UdpDatagram>,
    /// Datagram consumed by `poll_recv_ready` that hasn't been delivered yet.
    peeked: Option<UdpDatagram>,
}

impl BridgeUdpSocket {
    pub(crate) fn new(
        tx: mpsc::Sender<UdpDatagram>,
        rx: mpsc::Receiver<UdpDatagram>,
        local_addr: SocketAddr,
    ) -> Self {
        Self {
            tx: Mutex::new(PollSender::new(tx)),
            rx: Mutex::new(RxState {
                receiver: rx,
                peeked: None,
            }),
            local_addr,
            peer_addr: Mutex::new(None),
        }
    }

    // --- Connectionless (send_to / recv_from) ---

    /// Send a datagram to the specified address.
    pub async fn send_to(&self, buf: &[u8], target: SocketAddr) -> io::Result<usize> {
        let dg = UdpDatagram {
            payload: Bytes::copy_from_slice(buf),
            addr: target,
        };
        let tx = self.tx.lock().await;
        tx.get_ref()
            .ok_or_else(broken_pipe)?
            .send(dg)
            .await
            .map_err(|_| broken_pipe())?;
        Ok(buf.len())
    }

    /// Receive a datagram, returning the number of bytes read and the source address.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let mut rx = self.rx.lock().await;
        let dg = if let Some(dg) = rx.peeked.take() {
            dg
        } else {
            rx.receiver.recv().await.ok_or_else(connection_reset)?
        };
        let n = std::cmp::min(buf.len(), dg.payload.len());
        buf[..n].copy_from_slice(&dg.payload[..n]);
        Ok((n, dg.addr))
    }

    /// Attempt to send a datagram without blocking.
    /// Returns `Err(WouldBlock)` if the channel is full.
    pub fn try_send_to(&self, buf: &[u8], target: SocketAddr) -> io::Result<usize> {
        let dg = UdpDatagram {
            payload: Bytes::copy_from_slice(buf),
            addr: target,
        };
        let tx = self.tx.try_lock().map_err(|_| would_block())?;
        tx.get_ref()
            .ok_or_else(broken_pipe)?
            .try_send(dg)
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => would_block(),
                mpsc::error::TrySendError::Closed(_) => broken_pipe(),
            })?;
        Ok(buf.len())
    }

    /// Attempt to receive a datagram without blocking.
    /// Returns `Err(WouldBlock)` if no datagrams are available.
    pub fn try_recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let mut rx = self.rx.try_lock().map_err(|_| would_block())?;
        let dg = if let Some(dg) = rx.peeked.take() {
            dg
        } else {
            rx.receiver.try_recv().map_err(|e| match e {
                mpsc::error::TryRecvError::Empty => would_block(),
                mpsc::error::TryRecvError::Disconnected => connection_reset(),
            })?
        };
        let n = std::cmp::min(buf.len(), dg.payload.len());
        buf[..n].copy_from_slice(&dg.payload[..n]);
        Ok((n, dg.addr))
    }

    // --- Poll variants ---

    /// Poll-based send_to.
    pub fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        let mut tx = match self.tx.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Poll::Pending,
        };

        match tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let dg = UdpDatagram {
                    payload: Bytes::copy_from_slice(buf),
                    addr: target,
                };
                tx.send_item(dg).map_err(|_| broken_pipe())?;
                Poll::Ready(Ok(buf.len()))
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(broken_pipe())),
            Poll::Pending => Poll::Pending,
        }
    }

    /// Poll-based recv_from.
    pub fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<SocketAddr>> {
        let mut rx = match self.rx.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Poll::Pending,
        };

        // Check peek buffer first
        if let Some(dg) = rx.peeked.take() {
            let n = std::cmp::min(buf.remaining(), dg.payload.len());
            buf.put_slice(&dg.payload[..n]);
            return Poll::Ready(Ok(dg.addr));
        }

        match rx.receiver.poll_recv(cx) {
            Poll::Ready(Some(dg)) => {
                let n = std::cmp::min(buf.remaining(), dg.payload.len());
                buf.put_slice(&dg.payload[..n]);
                Poll::Ready(Ok(dg.addr))
            }
            Poll::Ready(None) => Poll::Ready(Err(connection_reset())),
            Poll::Pending => Poll::Pending,
        }
    }

    /// Check send readiness.
    pub fn poll_send_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut tx = match self.tx.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Poll::Pending,
        };

        match tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                tx.abort_send();
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(broken_pipe())),
            Poll::Pending => Poll::Pending,
        }
    }

    /// Check recv readiness.
    pub fn poll_recv_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut rx = match self.rx.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Poll::Pending,
        };

        // Already have a peeked datagram — ready.
        if rx.peeked.is_some() {
            return Poll::Ready(Ok(()));
        }

        // Try to receive one datagram and stash it in the peek buffer.
        match rx.receiver.poll_recv(cx) {
            Poll::Ready(Some(dg)) => {
                rx.peeked = Some(dg);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Err(connection_reset())),
            Poll::Pending => Poll::Pending,
        }
    }

    // --- Connected mode ---

    /// Associate a default remote address.
    pub async fn connect(&self, addr: SocketAddr) -> io::Result<()> {
        let mut peer = self.peer_addr.lock().await;
        *peer = Some(addr);
        Ok(())
    }

    /// Send to the connected peer.
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        let peer = self.peer_addr.lock().await;
        let addr =
            peer.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))?;
        self.send_to(buf, addr).await
    }

    /// Receive the next datagram. Equivalent to `recv_from` but discards
    /// the source address.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        let (n, _addr) = self.recv_from(buf).await?;
        Ok(n)
    }

    /// Poll-based send to the connected peer.
    pub fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let peer = match self.peer_addr.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Poll::Pending,
        };
        let addr = match *peer {
            Some(a) => a,
            None => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "not connected",
                )));
            }
        };
        drop(peer); // release lock before calling poll_send_to
        self.poll_send_to(cx, buf, addr)
    }

    /// Poll-based recv from any source. Equivalent to `poll_recv_from` but
    /// discards the source address.
    pub fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.poll_recv_from(cx, buf) {
            Poll::Ready(Ok(_addr)) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    // --- Metadata ---

    /// Local address this socket is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local_addr)
    }

    /// Remote address, if `connect()` was called.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        let peer = self
            .peer_addr
            .try_lock()
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "lock contention"))?;
        peer.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))
    }
}

impl std::fmt::Debug for BridgeUdpSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeUdpSocket")
            .field("local_addr", &self.local_addr)
            .finish()
    }
}

// --- Address conversion helpers ---

/// Convert `std::net::SocketAddr` to `smoltcp::wire::IpEndpoint`.
pub(crate) fn to_smoltcp_endpoint(addr: SocketAddr) -> smoltcp::wire::IpEndpoint {
    let ip = match addr.ip() {
        IpAddr::V4(v4) => smoltcp::wire::IpAddress::Ipv4(v4),
        IpAddr::V6(_) => unimplemented!("IPv6 not supported by smoltcp config"),
    };
    smoltcp::wire::IpEndpoint::new(ip, addr.port())
}

/// Convert `smoltcp::wire::IpEndpoint` to `std::net::SocketAddr`.
pub(crate) fn from_smoltcp_endpoint(ep: smoltcp::wire::IpEndpoint) -> SocketAddr {
    let ip = match ep.addr {
        smoltcp::wire::IpAddress::Ipv4(v4) => IpAddr::V4(v4),
    };
    SocketAddr::new(ip, ep.port)
}

// --- Error helpers ---

fn broken_pipe() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "lcore relay task exited")
}

fn connection_reset() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, "lcore relay task exited")
}

fn would_block() -> io::Error {
    io::Error::from(io::ErrorKind::WouldBlock)
}

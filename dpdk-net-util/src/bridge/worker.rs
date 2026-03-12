use bytes::Bytes;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, mpsc, oneshot};

use arc_swap::ArcSwap;
use dpdk_net::runtime::ReactorHandle;
use dpdk_net::socket::{TcpListener, TcpStream, UdpSocket};

use super::command::{BridgeCommand, BridgeStreamChannels};
use super::error::BridgeError;
use super::listener::BridgeTcpListener;
use super::stream::BridgeTcpStream;
use super::udp::{BridgeUdpSocket, UdpDatagram, from_smoltcp_endpoint, to_smoltcp_endpoint};

/// Default channel capacity for data channels (OS ↔ lcore).
const DATA_CHANNEL_SIZE: usize = 256;

/// Default channel capacity for command channels.
const CMD_CHANNEL_SIZE: usize = 1024;

/// Default channel capacity for the accept channel (listener → OS).
const ACCEPT_CHANNEL_SIZE: usize = 64;

/// Default channel capacity for UDP datagram channels (per direction).
const UDP_CHANNEL_SIZE: usize = 1024;

/// Default buffer sizes for TCP sockets created by the bridge.
const RX_BUF_SIZE: usize = 65536;
const TX_BUF_SIZE: usize = 65536;

/// Default UDP socket buffer parameters.
const UDP_RX_PACKETS: usize = 256;
const UDP_TX_PACKETS: usize = 256;
const UDP_MAX_PACKET_SIZE: usize = 1500;

/// Shared registry that tracks which lcores are available.
pub(crate) struct WorkerRegistry {
    /// Registered worker channels, appended to by `spawn()`.
    pub workers: ArcSwap<Vec<mpsc::Sender<BridgeCommand>>>,
    /// Round-robin counter for queue selection.
    pub next: AtomicUsize,
    /// Notification for OS threads waiting for workers to register.
    pub ready: Notify,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self {
            workers: ArcSwap::from_pointee(Vec::new()),
            next: AtomicUsize::new(0),
            ready: Notify::new(),
        }
    }

    /// Pick the next worker channel using round-robin.
    pub fn select(&self) -> Option<mpsc::Sender<BridgeCommand>> {
        let workers = self.workers.load();
        if workers.is_empty() {
            return None;
        }
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % workers.len();
        Some(workers[idx].clone())
    }
}

/// Lcore-side handle for spawning bridge workers.
///
/// Captured in the `DpdkApp::run()` closure. `Send + Sync` so it can cross
/// the thread boundary into the lcore closure.
#[derive(Clone)]
pub struct BridgeWorkers {
    pub(crate) registry: Arc<WorkerRegistry>,
}

impl BridgeWorkers {
    /// Spawn a bridge worker on the current lcore.
    ///
    /// Creates a command channel, registers it with the shared registry,
    /// and spawns a `bridge_worker` task on the current `LocalSet`.
    /// Must be called from within an async context on an lcore
    /// (i.e., inside the `DpdkApp::run` closure).
    pub fn spawn(&self, reactor: &ReactorHandle) {
        let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL_SIZE);

        // Register this worker's channel with the shared registry
        self.registry.workers.rcu(|current| {
            let mut new = (**current).clone();
            new.push(cmd_tx.clone());
            new
        });
        self.registry.ready.notify_waiters();

        let reactor = reactor.clone();
        tokio::task::spawn_local(async move {
            bridge_worker(reactor, cmd_rx).await;
        });
    }
}

/// Simple per-worker ephemeral port allocator.
///
/// Tracks in-use ports within the IANA ephemeral range (49152–65535).
struct EphemeralPorts {
    next: u16,
}

impl EphemeralPorts {
    fn new() -> Self {
        Self { next: 49152 }
    }

    fn allocate(&mut self) -> u16 {
        let port = self.next;
        // Wrap around within the ephemeral range
        self.next = if self.next == 65535 {
            49152
        } else {
            self.next + 1
        };
        port
    }
}

/// Main bridge worker loop. Spawned as a `spawn_local` task on each lcore.
async fn bridge_worker(reactor: ReactorHandle, mut cmd_rx: mpsc::Receiver<BridgeCommand>) {
    let mut ports = EphemeralPorts::new();

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            BridgeCommand::Connect {
                remote_addr,
                remote_port,
                reply_tx,
            } => {
                let local_port = ports.allocate();

                let stream = match TcpStream::connect(
                    &reactor,
                    remote_addr,
                    remote_port,
                    local_port,
                    RX_BUF_SIZE,
                    TX_BUF_SIZE,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = reply_tx.send(Err(e.into()));
                        continue;
                    }
                };

                if stream.wait_connected().await.is_err() {
                    let _ = reply_tx.send(Err(BridgeError::ConnectionFailed));
                    continue;
                }

                // Create bidirectional relay channels
                let (tx_to_lcore, rx_from_os) = mpsc::channel(DATA_CHANNEL_SIZE);
                let (tx_to_os, rx_from_lcore) = mpsc::channel(DATA_CHANNEL_SIZE);
                let (close_tx, close_rx) = oneshot::channel();

                let _ = reply_tx.send(Ok(BridgeStreamChannels {
                    data_tx: tx_to_lcore,
                    data_rx: rx_from_lcore,
                    close_tx,
                }));

                tokio::task::spawn_local(relay_task(stream, rx_from_os, tx_to_os, close_rx));
            }
            BridgeCommand::Listen { port, reply_tx } => {
                let listener = match TcpListener::bind(&reactor, port, RX_BUF_SIZE, TX_BUF_SIZE) {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = reply_tx.send(Err(e.into()));
                        continue;
                    }
                };

                let (accept_tx, accept_rx) = mpsc::channel(ACCEPT_CHANNEL_SIZE);
                let _ = reply_tx.send(Ok(BridgeTcpListener { accept_rx }));

                // Spawn accept loop — each accepted connection gets its own relay task
                tokio::task::spawn_local(accept_loop(listener, accept_tx));
            }
            BridgeCommand::BindUdp { port, reply_tx } => {
                let socket = match UdpSocket::bind(
                    &reactor,
                    port,
                    UDP_RX_PACKETS,
                    UDP_TX_PACKETS,
                    UDP_MAX_PACKET_SIZE,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = reply_tx.send(Err(e.into()));
                        continue;
                    }
                };

                // Resolve local address from the reactor's IP config + bound port.
                let local_ip = reactor.ip_addr().unwrap_or(smoltcp::wire::IpAddress::Ipv4(
                    smoltcp::wire::Ipv4Address::UNSPECIFIED,
                ));
                let local_addr = SocketAddr::new(
                    match local_ip {
                        smoltcp::wire::IpAddress::Ipv4(v4) => IpAddr::V4(v4),
                    },
                    port,
                );

                let (tx_to_lcore, rx_from_os) = mpsc::channel(UDP_CHANNEL_SIZE);
                let (tx_to_os, rx_from_lcore) = mpsc::channel(UDP_CHANNEL_SIZE);

                let bridge_socket = BridgeUdpSocket::new(tx_to_lcore, rx_from_lcore, local_addr);
                let _ = reply_tx.send(Ok(bridge_socket));

                tokio::task::spawn_local(udp_relay_task(socket, rx_from_os, tx_to_os));
            }
        }
    }
}

/// Accept loop for a bridge listener. Runs on the lcore's `LocalSet`.
async fn accept_loop(
    mut listener: TcpListener,
    accept_tx: mpsc::Sender<Result<BridgeTcpStream, BridgeError>>,
) {
    loop {
        let stream = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                let _ = accept_tx.send(Err(e.into())).await;
                continue;
            }
        };

        let (tx_to_lcore, rx_from_os) = mpsc::channel(DATA_CHANNEL_SIZE);
        let (tx_to_os, rx_from_lcore) = mpsc::channel(DATA_CHANNEL_SIZE);
        let (close_tx, close_rx) = oneshot::channel();

        let bridge_stream = BridgeTcpStream::new(tx_to_lcore, rx_from_lcore, close_tx);

        if accept_tx.send(Ok(bridge_stream)).await.is_err() {
            // OS side dropped the listener handle
            break;
        }

        tokio::task::spawn_local(relay_task(stream, rx_from_os, tx_to_os, close_rx));
    }
}

/// Per-connection relay task. Runs on the lcore's `LocalSet`.
///
/// Owns the real `TcpStream` (!Send) and shuttles data between it and
/// the Send channels connected to the OS thread's `BridgeTcpStream`.
async fn relay_task(
    stream: TcpStream,
    mut rx_from_os: mpsc::Receiver<Bytes>,
    tx_to_os: mpsc::Sender<Result<Bytes, BridgeError>>,
    close_rx: oneshot::Receiver<()>,
) {
    let mut recv_buf = vec![0u8; 65536];
    tokio::pin!(close_rx);

    loop {
        tokio::select! {
            // Forward data from DPDK → OS thread (ingress)
            result = stream.recv(&mut recv_buf) => {
                match result {
                    Ok(0) => {
                        // EOF — send empty chunk to signal closure
                        let _ = tx_to_os.send(Ok(Bytes::new())).await;
                        break;
                    }
                    Ok(n) => {
                        let chunk = Bytes::copy_from_slice(&recv_buf[..n]);
                        if tx_to_os.send(Ok(chunk)).await.is_err() {
                            break; // OS side dropped
                        }
                    }
                    Err(e) => {
                        let _ = tx_to_os.send(Err(e.into())).await;
                        break;
                    }
                }
            }
            // Forward data from OS thread → DPDK (egress)
            data = rx_from_os.recv() => {
                match data {
                    Some(bytes) => {
                        if let Err(e) = stream.send(&bytes).await {
                            let _ = tx_to_os.send(Err(e.into())).await;
                            break;
                        }
                    }
                    None => break, // OS side dropped the write half
                }
            }
            // Graceful shutdown requested
            _ = &mut close_rx => {
                let _ = stream.close().await;
                break;
            }
        }
    }
}

/// Per-port UDP relay task. Runs on the lcore's `LocalSet`.
///
/// Owns the real `UdpSocket` (!Send) and shuttles datagrams between it and
/// the Send channels connected to the OS thread's `BridgeUdpSocket`.
async fn udp_relay_task(
    socket: UdpSocket,
    mut rx_from_os: mpsc::Receiver<UdpDatagram>,
    tx_to_os: mpsc::Sender<UdpDatagram>,
) {
    let mut recv_buf = vec![0u8; UDP_MAX_PACKET_SIZE];

    loop {
        tokio::select! {
            // Egress: OS thread → NIC
            datagram = rx_from_os.recv() => {
                let Some(dg) = datagram else { break }; // OS side dropped
                let endpoint = to_smoltcp_endpoint(dg.addr);
                // send_slice errors (Unaddressable, BufferFull) are silently dropped.
                // The OS side already got Ok(len) when the datagram entered the channel.
                let _ = socket.send_to(&dg.payload, endpoint).await;
            }
            // Ingress: NIC → OS thread
            result = socket.recv_from(&mut recv_buf) => {
                match result {
                    Ok((len, metadata)) => {
                        let dg = UdpDatagram {
                            payload: Bytes::copy_from_slice(&recv_buf[..len]),
                            addr: from_smoltcp_endpoint(metadata.endpoint),
                        };
                        // Drop on full — best-effort, consistent with UDP semantics.
                        // Using try_send (not send.await) so ingress is never blocked
                        // by a slow OS consumer.
                        let _ = tx_to_os.try_send(dg);
                    }
                    Err(_) => continue, // transient recv error
                }
            }
        }
    }
}

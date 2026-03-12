use bytes::Bytes;
use smoltcp::wire::IpAddress;
use tokio::sync::{mpsc, oneshot};

use super::error::BridgeError;
use super::listener::BridgeTcpListener;
use super::udp::BridgeUdpSocket;

/// Command sent from OS threads to an lcore bridge worker.
pub(crate) enum BridgeCommand {
    /// Open a new outbound TCP connection.
    Connect {
        remote_addr: IpAddress,
        remote_port: u16,
        reply_tx: oneshot::Sender<Result<BridgeStreamChannels, BridgeError>>,
    },
    /// Bind a TCP listener on the DPDK stack.
    Listen {
        port: u16,
        reply_tx: oneshot::Sender<Result<BridgeTcpListener, BridgeError>>,
    },
    /// Bind a UDP socket on the DPDK stack.
    BindUdp {
        port: u16,
        reply_tx: oneshot::Sender<Result<BridgeUdpSocket, BridgeError>>,
    },
}

/// Channels returned to the OS thread after a successful connect.
pub(crate) struct BridgeStreamChannels {
    /// Send data to the lcore relay task (OS → lcore).
    pub data_tx: mpsc::Sender<Bytes>,
    /// Receive data from the lcore relay task (lcore → OS).
    pub data_rx: mpsc::Receiver<Result<Bytes, BridgeError>>,
    /// Signal graceful close to the relay task.
    pub close_tx: oneshot::Sender<()>,
}

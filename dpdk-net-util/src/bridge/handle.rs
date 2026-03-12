use std::sync::Arc;

use smoltcp::wire::IpAddress;
use tokio::sync::oneshot;

use super::command::BridgeCommand;
use super::error::BridgeError;
use super::listener::BridgeTcpListener;
use super::stream::BridgeTcpStream;
use super::udp::BridgeUdpSocket;
use super::worker::{BridgeWorkers, WorkerRegistry};

/// Handle for OS threads to request TCP connections through DPDK.
///
/// This type is `Send + Sync` — safe to share across OS threads via `Clone`.
///
/// Created via [`DpdkBridge::pair()`], which also returns a [`BridgeWorkers`]
/// half that must be passed into the `DpdkApp::run()` closure.
#[derive(Clone)]
pub struct DpdkBridge {
    registry: Arc<WorkerRegistry>,
}

impl DpdkBridge {
    /// Create a linked pair: bridge handle (for OS threads) + worker
    /// config (for the `DpdkApp::run` closure).
    pub fn pair() -> (DpdkBridge, BridgeWorkers) {
        let registry = Arc::new(WorkerRegistry::new());
        (
            DpdkBridge {
                registry: registry.clone(),
            },
            BridgeWorkers { registry },
        )
    }

    /// Connect to a remote address through the DPDK stack.
    ///
    /// Awaits until the TCP handshake completes on the lcore.
    pub async fn connect(
        &self,
        remote_addr: IpAddress,
        remote_port: u16,
    ) -> Result<BridgeTcpStream, BridgeError> {
        let worker = self.registry.select().ok_or(BridgeError::Disconnected)?;

        let (reply_tx, reply_rx) = oneshot::channel();

        worker
            .send(BridgeCommand::Connect {
                remote_addr,
                remote_port,
                reply_tx,
            })
            .await
            .map_err(|_| BridgeError::Disconnected)?;

        let channels = reply_rx.await.map_err(|_| BridgeError::Disconnected)??;

        Ok(BridgeTcpStream::new(
            channels.data_tx,
            channels.data_rx,
            channels.close_tx,
        ))
    }

    /// Bind a TCP listener on the DPDK stack.
    ///
    /// Accepted connections are returned as [`BridgeTcpStream`]s.
    pub async fn listen(&self, port: u16) -> Result<BridgeTcpListener, BridgeError> {
        let worker = self.registry.select().ok_or(BridgeError::Disconnected)?;

        let (reply_tx, reply_rx) = oneshot::channel();

        worker
            .send(BridgeCommand::Listen { port, reply_tx })
            .await
            .map_err(|_| BridgeError::Disconnected)?;

        reply_rx.await.map_err(|_| BridgeError::Disconnected)?
    }

    /// Bind a UDP socket on the DPDK stack.
    ///
    /// Returns a `BridgeUdpSocket` that mirrors `tokio::net::UdpSocket` API.
    pub async fn bind_udp(&self, port: u16) -> Result<BridgeUdpSocket, BridgeError> {
        let worker = self.registry.select().ok_or(BridgeError::Disconnected)?;

        let (reply_tx, reply_rx) = oneshot::channel();

        worker
            .send(BridgeCommand::BindUdp { port, reply_tx })
            .await
            .map_err(|_| BridgeError::Disconnected)?;

        reply_rx.await.map_err(|_| BridgeError::Disconnected)?
    }

    /// Wait until at least one lcore worker has registered.
    ///
    /// Useful for OS threads that need to call `connect()` immediately
    /// after spawning, before the lcore has had a chance to register.
    pub async fn wait_ready(&self) {
        loop {
            if !self.registry.workers.load().is_empty() {
                return;
            }
            self.registry.ready.notified().await;
        }
    }
}

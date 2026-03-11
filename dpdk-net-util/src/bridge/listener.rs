use tokio::sync::mpsc;

use super::error::BridgeError;
use super::stream::BridgeTcpStream;

/// A TCP listener proxied through DPDK. `Send`, `!Sync`.
///
/// Accepted connections are returned as [`BridgeTcpStream`]s.
pub struct BridgeTcpListener {
    pub(crate) accept_rx: mpsc::Receiver<Result<BridgeTcpStream, BridgeError>>,
}

impl BridgeTcpListener {
    /// Accept the next incoming connection.
    pub async fn accept(&mut self) -> Result<BridgeTcpStream, BridgeError> {
        self.accept_rx
            .recv()
            .await
            .ok_or(BridgeError::Disconnected)?
    }
}

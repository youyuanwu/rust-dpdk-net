use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use quinn::{AsyncTimer, AsyncUdpSocket, Runtime};

use super::DpdkQuinnSocket;
use crate::bridge::{BridgeError, DpdkBridge};

/// Quinn [`Runtime`] backed by DPDK via [`DpdkBridge`].
///
/// Timers and task spawning delegate to tokio; UDP sockets bypass the kernel
/// and run over the DPDK userspace stack through [`DpdkQuinnSocket`].
///
/// Use [`endpoint()`](Self::endpoint) to create a [`quinn::Endpoint`] —
/// `wrap_udp_socket` is deliberately unsupported because Quinn would hand us
/// an OS socket we don't want.
#[derive(Clone)]
pub struct DpdkQuinnRuntime {
    bridge: DpdkBridge,
}

impl std::fmt::Debug for DpdkQuinnRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DpdkQuinnRuntime").finish_non_exhaustive()
    }
}

impl DpdkQuinnRuntime {
    pub fn new(bridge: DpdkBridge) -> Self {
        Self { bridge }
    }

    /// Create a Quinn endpoint bound to `port` on the DPDK stack.
    ///
    /// This bypasses [`Runtime::wrap_udp_socket`] by constructing the endpoint
    /// with [`quinn::Endpoint::new_with_abstract_socket`].
    pub async fn endpoint(
        &self,
        config: quinn::EndpointConfig,
        server_config: Option<quinn::ServerConfig>,
        port: u16,
    ) -> Result<quinn::Endpoint, BridgeError> {
        let bridge_socket = self.bridge.bind_udp(port).await?;
        let quinn_socket: Arc<dyn AsyncUdpSocket> = Arc::new(DpdkQuinnSocket::new(bridge_socket));
        let runtime: Arc<dyn Runtime> = Arc::new(self.clone());
        quinn::Endpoint::new_with_abstract_socket(config, server_config, quinn_socket, runtime)
            .map_err(BridgeError::Io)
    }
}

impl Runtime for DpdkQuinnRuntime {
    fn new_timer(&self, i: Instant) -> Pin<Box<dyn AsyncTimer>> {
        Box::pin(tokio::time::sleep_until(i.into()))
    }

    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        tokio::spawn(future);
    }

    fn wrap_udp_socket(&self, _sock: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "use DpdkQuinnRuntime::endpoint() instead",
        ))
    }
}

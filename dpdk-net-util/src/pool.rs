use std::collections::HashMap;

use bytes::Bytes;
use hyper::body::Incoming;
use hyper::{Request, Response};

use dpdk_net::runtime::ReactorHandle;
use smoltcp::wire::IpAddress;

use crate::client::ClientConfig;
use crate::connection::{Connection, HttpVersion};
use crate::error::Error;

/// Simple per-host connection pool.
///
/// Maintains idle connections keyed by `(IpAddress, port)` and reuses them
/// for subsequent requests. Connections that are no longer ready are
/// discarded automatically.
///
/// # `!Send`
/// This type is `!Send`. Use one pool per lcore.
///
/// # Examples
///
/// ```ignore
/// use dpdk_net_util::ConnectionPool;
/// use dpdk_net::runtime::ReactorHandle;
///
/// async fn run(reactor: &ReactorHandle) {
///     let mut pool = ConnectionPool::new(reactor.clone());
///     // Connections are created on first use and reused after.
/// }
/// ```
pub struct ConnectionPool {
    reactor: ReactorHandle,
    config: ClientConfig,
    connections: HashMap<(IpAddress, u16), Vec<Connection>>,
    max_idle_per_host: usize,
}

impl ConnectionPool {
    /// Create a pool with default configuration and up to 8 idle connections
    /// per host.
    pub fn new(reactor: ReactorHandle) -> Self {
        Self::with_config(reactor, ClientConfig::default(), 8)
    }

    /// Create a pool with custom configuration.
    pub fn with_config(
        reactor: ReactorHandle,
        config: ClientConfig,
        max_idle_per_host: usize,
    ) -> Self {
        Self {
            reactor,
            config,
            connections: HashMap::new(),
            max_idle_per_host,
        }
    }

    /// Acquire a ready connection to the given host, or create one.
    ///
    /// `local_port` is used only when creating a new connection.
    pub async fn connection(
        &mut self,
        addr: IpAddress,
        port: u16,
        local_port: u16,
    ) -> Result<&mut Connection, Error> {
        let key = (addr, port);

        // Check for an existing ready connection (immutable borrow, dropped before mutation).
        let has_ready = self
            .connections
            .get(&key)
            .is_some_and(|conns| conns.iter().any(|c| c.is_ready()));

        if has_ready {
            let conns = self.connections.get_mut(&key).unwrap();
            let pos = conns.iter().position(|c| c.is_ready()).unwrap();
            return Ok(&mut conns[pos]);
        }

        // Create a new connection.
        let conn = match self.config.http_version {
            HttpVersion::Http1 => {
                Connection::http1(
                    &self.reactor,
                    addr,
                    port,
                    local_port,
                    self.config.rx_buffer_size,
                    self.config.tx_buffer_size,
                )
                .await?
            }
            HttpVersion::Http2 => {
                Connection::http2(
                    &self.reactor,
                    addr,
                    port,
                    local_port,
                    self.config.rx_buffer_size,
                    self.config.tx_buffer_size,
                )
                .await?
            }
        };

        let conns = self.connections.entry(key).or_default();

        // Enforce limit by removing oldest idle connection.
        if conns.len() >= self.max_idle_per_host {
            conns.remove(0);
        }

        conns.push(conn);
        Ok(conns.last_mut().unwrap())
    }

    /// Send a one-shot request, reusing a pooled connection if available.
    pub async fn request<B>(
        &mut self,
        addr: IpAddress,
        port: u16,
        local_port: u16,
        request: Request<B>,
    ) -> Result<Response<Incoming>, Error>
    where
        B: hyper::body::Body<Data = Bytes> + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let conn = self.connection(addr, port, local_port).await?;
        conn.send_request(request).await
    }

    /// Remove all idle connections.
    pub fn clear(&mut self) {
        self.connections.clear();
    }
}

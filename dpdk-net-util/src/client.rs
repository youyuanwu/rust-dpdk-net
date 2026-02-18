use std::time::Duration;

use bytes::Bytes;
use hyper::body::Incoming;
use hyper::{Request, Response};

use dpdk_net::runtime::ReactorHandle;
use smoltcp::wire::IpAddress;

use crate::connection::{Connection, HttpVersion};
use crate::error::Error;

/// Configuration for [`DpdkHttpClient`].
pub struct ClientConfig {
    /// Receive buffer size for TCP connections (bytes).
    pub rx_buffer_size: usize,
    /// Transmit buffer size for TCP connections (bytes).
    pub tx_buffer_size: usize,
    /// HTTP version preference.
    pub http_version: HttpVersion,
    /// Connection timeout (not yet enforced — reserved for future use).
    pub connect_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            rx_buffer_size: 16384,
            tx_buffer_size: 16384,
            http_version: HttpVersion::Http1,
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// HTTP client for DPDK networks.
///
/// Wraps hyper's low-level connection API, handling TCP connection setup
/// and HTTP handshake. Each client is bound to a single reactor (lcore).
///
/// # `!Send`
/// This type is `!Send` because it holds a [`ReactorHandle`] containing
/// `Rc<RefCell<...>>`. All usage must be on a single lcore.
///
/// # Examples
///
/// ```ignore
/// use dpdk_net_util::{DpdkHttpClient, ClientConfig};
/// use dpdk_net::runtime::ReactorHandle;
/// use smoltcp::wire::IpAddress;
///
/// async fn run(reactor: &ReactorHandle) {
///     let client = DpdkHttpClient::new(reactor.clone());
///     let mut conn = client
///         .connect(IpAddress::v4(10, 0, 0, 1), 8080, 1234)
///         .await
///         .unwrap();
///
///     let req = hyper::Request::get("/")
///         .header("Host", "10.0.0.1:8080")
///         .body(http_body_util::Empty::<bytes::Bytes>::new())
///         .unwrap();
///     let resp = conn.send_request(req).await.unwrap();
/// }
/// ```
pub struct DpdkHttpClient {
    reactor: ReactorHandle,
    config: ClientConfig,
}

impl DpdkHttpClient {
    /// Create a new HTTP client with default configuration.
    pub fn new(reactor: ReactorHandle) -> Self {
        Self::with_config(reactor, ClientConfig::default())
    }

    /// Create a new HTTP client with custom configuration.
    pub fn with_config(reactor: ReactorHandle, config: ClientConfig) -> Self {
        Self { reactor, config }
    }

    /// Open an HTTP connection to the given address and port.
    ///
    /// `local_port` is the ephemeral source port for the TCP connection.
    /// The HTTP version is determined by [`ClientConfig::http_version`].
    pub async fn connect(
        &self,
        addr: IpAddress,
        port: u16,
        local_port: u16,
    ) -> Result<Connection, Error> {
        match self.config.http_version {
            HttpVersion::Http1 => {
                Connection::http1(
                    &self.reactor,
                    addr,
                    port,
                    local_port,
                    self.config.rx_buffer_size,
                    self.config.tx_buffer_size,
                )
                .await
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
                .await
            }
        }
    }

    /// Send a one-shot HTTP request, creating a new connection.
    ///
    /// For multiple requests to the same host, prefer [`connect`](Self::connect)
    /// to reuse the connection.
    pub async fn request<B>(
        &self,
        addr: IpAddress,
        port: u16,
        local_port: u16,
        request: Request<B>,
    ) -> Result<Response<Incoming>, Error>
    where
        B: hyper::body::Body<Data = Bytes> + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let mut conn = self.connect(addr, port, local_port).await?;
        conn.send_request(request).await
    }

    /// Returns a reference to the client configuration.
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }
}

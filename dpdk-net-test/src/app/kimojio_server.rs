//! HTTP/1.1 server implementation for kimojio (io_uring-based async runtime).
//!
//! This module provides an HTTP/1.1 server that uses kimojio's completion-based
//! I/O model with io_uring. Unlike tokio's readiness-based model, kimojio uses
//! completion-based I/O where operations complete with data already available.
//!
//! # Key Differences from Tokio-based Server
//!
//! - Uses `AsyncStreamRead`/`AsyncStreamWrite` traits instead of tokio's `AsyncRead`/`AsyncWrite`
//! - `try_read` returns bytes read directly (not just readiness notification)
//! - `read` fills the entire buffer before returning
//! - `write` writes the entire buffer before returning
//! - Uses `OwnedFdStream` for socket I/O
//!
//! # Example
//!
//! ```no_run
//! use kimojio::{Errno, operations::{accept, spawn_task}, socket_helpers::create_server_socket};
//!
//! #[kimojio::main]
//! async fn main() -> Result<(), Errno> {
//!     // Server setup would go here
//!     Ok(())
//! }
//! ```

use std::fmt;
use std::future::Future;

use hyper::body::Bytes;
use hyper::header;
use hyper::http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Version,
};

/// Maximum number of headers to parse
const MAX_HEADERS: usize = 100;

/// Initial buffer size for reading headers
const INITIAL_BUF_SIZE: usize = 1024;

/// Maximum buffer size for headers (64 KB)
const MAX_BUF_SIZE: usize = 64 * 1024;

/// Error type for HTTP parsing
#[derive(Debug)]
pub enum ParseError {
    /// I/O error during reading
    Io(String),
    /// httparse error
    Parse(httparse::Error),
    /// Invalid HTTP method
    InvalidMethod,
    /// Invalid HTTP version
    InvalidVersion,
    /// Invalid header
    InvalidHeader,
    /// Invalid URI
    InvalidUri,
    /// Headers too large
    HeadersTooLarge,
    /// Invalid Content-Length
    InvalidContentLength,
    /// Connection closed
    ConnectionClosed,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "I/O error: {}", e),
            ParseError::Parse(e) => write!(f, "Parse error: {}", e),
            ParseError::InvalidMethod => write!(f, "Invalid HTTP method"),
            ParseError::InvalidVersion => write!(f, "Invalid HTTP version"),
            ParseError::InvalidHeader => write!(f, "Invalid header format"),
            ParseError::InvalidUri => write!(f, "Invalid URI"),
            ParseError::HeadersTooLarge => write!(f, "Headers too large"),
            ParseError::InvalidContentLength => write!(f, "Invalid Content-Length"),
            ParseError::ConnectionClosed => write!(f, "Connection closed"),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<httparse::Error> for ParseError {
    fn from(e: httparse::Error) -> Self {
        ParseError::Parse(e)
    }
}

/// Create a server socket with SO_REUSEPORT for thread-per-core architecture.
///
/// This is similar to kimojio's `create_server_socket` but adds SO_REUSEPORT
/// which allows multiple threads to bind to the same port for load balancing.
async fn create_server_socket_reuseport(port: u16) -> Result<kimojio::OwnedFd, kimojio::Errno> {
    use kimojio::operations::{AddressFamily, SocketType, ipproto, listen, socket};
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};

    let server_fd = socket(AddressFamily::INET6, SocketType::STREAM, Some(ipproto::TCP)).await?;

    // Use socket2 to set socket options since we have it as a dependency
    // This is safe because we're just setting options on the fd we own
    let socket2_socket = unsafe { socket2::Socket::from_raw_fd(server_fd.as_raw_fd()) };
    socket2_socket.set_tcp_nodelay(true).ok();
    socket2_socket.set_reuse_address(true).ok();
    // SO_REUSEPORT allows multiple threads to bind to the same port
    socket2_socket.set_reuse_port(true).ok();

    // Bind to [::]:port to listen for connections on any interface
    let addr: std::net::SocketAddr = format!("[::]:{}", port).parse().unwrap();
    socket2_socket
        .bind(&addr.into())
        .map_err(|_| kimojio::Errno::ADDRINUSE)?;

    // Consume socket2_socket without closing the fd (server_fd still owns it)
    let _ = socket2_socket.into_raw_fd();

    listen(&server_fd, nix::libc::SOMAXCONN)?;

    Ok(server_fd)
}

/// Get Content-Length from headers.
fn get_content_length(headers: &HeaderMap) -> Option<usize> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

/// Check if Transfer-Encoding is chunked.
fn is_chunked(headers: &HeaderMap) -> bool {
    headers
        .get(header::TRANSFER_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
}

/// Check if connection should be kept alive.
pub fn should_keep_alive(headers: &HeaderMap, version: Version) -> bool {
    match headers.get(header::CONNECTION) {
        Some(conn) => conn
            .to_str()
            .map(|s| s.eq_ignore_ascii_case("keep-alive"))
            .unwrap_or(false),
        None => version == Version::HTTP_11,
    }
}

/// Get version string for serialization.
fn version_str(version: Version) -> &'static str {
    match version {
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2.0",
        Version::HTTP_3 => "HTTP/3.0",
        _ => "HTTP/1.1",
    }
}

/// Get reason phrase for a status code.
fn reason_phrase(status: StatusCode) -> &'static str {
    status.canonical_reason().unwrap_or("Unknown")
}

/// HTTP request parser for kimojio's completion-based I/O.
///
/// This parser is designed for completion-based I/O where reads return
/// the actual data rather than just signaling readiness. It uses `httparse`
/// for the actual HTTP parsing.
pub struct KimojioHttpParser {
    buf: Vec<u8>,
    /// Number of valid bytes in the buffer
    len: usize,
}

impl KimojioHttpParser {
    /// Create a new parser.
    pub fn new() -> Self {
        Self {
            buf: vec![0u8; INITIAL_BUF_SIZE],
            len: 0,
        }
    }

    /// Parse an HTTP request from the stream using kimojio's AsyncStreamRead.
    ///
    /// This method uses completion-based I/O - each read operation completes
    /// with data already available in the buffer.
    ///
    /// Returns `None` if the connection was closed cleanly before any data was received.
    pub async fn parse_request<R>(
        &mut self,
        reader: &mut R,
    ) -> Result<Option<Request<Bytes>>, ParseError>
    where
        R: KimojioAsyncRead,
    {
        // Read and parse headers
        let (method, uri, version, headers, header_len) = match self.parse_headers(reader).await? {
            Some(parsed) => parsed,
            None => return Ok(None),
        };

        // Remove parsed headers from buffer, keeping any leftover body data
        self.buf.copy_within(header_len..self.len, 0);
        self.len -= header_len;

        // Read body based on Content-Length or Transfer-Encoding
        let body = self.read_body(reader, &headers).await?;

        // Build the request
        let mut builder = Request::builder().method(method).uri(uri).version(version);

        if let Some(h) = builder.headers_mut() {
            *h = headers;
        }

        let request = builder
            .body(Bytes::from(body))
            .map_err(|_| ParseError::InvalidUri)?;

        Ok(Some(request))
    }

    /// Read data until headers are complete and parse them.
    async fn parse_headers<R>(
        &mut self,
        reader: &mut R,
    ) -> Result<Option<(Method, String, Version, HeaderMap, usize)>, ParseError>
    where
        R: KimojioAsyncRead,
    {
        loop {
            // Try to parse with current buffer
            let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
            let mut req = httparse::Request::new(&mut headers);

            match req.parse(&self.buf[..self.len])? {
                httparse::Status::Complete(header_len) => {
                    // Successfully parsed headers
                    let method = Method::from_bytes(req.method.unwrap_or("").as_bytes())
                        .map_err(|_| ParseError::InvalidMethod)?;

                    let uri = req.path.unwrap_or("/").to_string();

                    let version = match req.version {
                        Some(0) => Version::HTTP_10,
                        Some(1) => Version::HTTP_11,
                        _ => return Err(ParseError::InvalidVersion),
                    };

                    // Convert headers to HeaderMap
                    let mut header_map = HeaderMap::new();
                    for h in req.headers.iter() {
                        let name = HeaderName::from_bytes(h.name.as_bytes())
                            .map_err(|_| ParseError::InvalidHeader)?;
                        let value = HeaderValue::from_bytes(h.value)
                            .map_err(|_| ParseError::InvalidHeader)?;
                        header_map.insert(name, value);
                    }

                    return Ok(Some((method, uri, version, header_map, header_len)));
                }
                httparse::Status::Partial => {
                    // Need more data
                    if self.len == 0 {
                        // First read - check if connection is closed
                        let n = self.read_more(reader).await?;
                        if n == 0 {
                            return Ok(None); // Clean close before any data
                        }
                    } else {
                        // Continue reading
                        let n = self.read_more(reader).await?;
                        if n == 0 {
                            return Err(ParseError::ConnectionClosed);
                        }
                    }
                }
            }
        }
    }

    /// Read more data into the buffer using kimojio's try_read.
    async fn read_more<R>(&mut self, reader: &mut R) -> Result<usize, ParseError>
    where
        R: KimojioAsyncRead,
    {
        // Grow buffer if needed
        if self.len == self.buf.len() {
            if self.buf.len() >= MAX_BUF_SIZE {
                return Err(ParseError::HeadersTooLarge);
            }
            self.buf.resize(self.buf.len() * 2, 0);
        }

        // Completion-based read - returns the number of bytes read
        let n = reader.try_read(&mut self.buf[self.len..]).await?;
        self.len += n;
        Ok(n)
    }

    /// Read the request body.
    async fn read_body<R>(
        &mut self,
        reader: &mut R,
        headers: &HeaderMap,
    ) -> Result<Vec<u8>, ParseError>
    where
        R: KimojioAsyncRead,
    {
        if is_chunked(headers) {
            return self.read_chunked_body(reader).await;
        }

        if let Some(content_length) = get_content_length(headers) {
            return self.read_fixed_body(reader, content_length).await;
        }

        Ok(Vec::new())
    }

    /// Read a fixed-size body.
    async fn read_fixed_body<R>(
        &mut self,
        reader: &mut R,
        length: usize,
    ) -> Result<Vec<u8>, ParseError>
    where
        R: KimojioAsyncRead,
    {
        let mut body = Vec::with_capacity(length);

        // Use any data already in buffer
        let from_buf = self.len.min(length);
        body.extend_from_slice(&self.buf[..from_buf]);
        self.buf.copy_within(from_buf..self.len, 0);
        self.len -= from_buf;

        // Read remaining using completion-based read
        while body.len() < length {
            let remaining = length - body.len();
            let mut chunk = vec![0u8; remaining.min(8192)];
            let n = reader.try_read(&mut chunk).await?;
            if n == 0 {
                return Err(ParseError::ConnectionClosed);
            }
            body.extend_from_slice(&chunk[..n]);
        }

        Ok(body)
    }

    /// Read a chunked transfer-encoded body.
    async fn read_chunked_body<R>(&mut self, reader: &mut R) -> Result<Vec<u8>, ParseError>
    where
        R: KimojioAsyncRead,
    {
        let mut body = Vec::new();

        loop {
            // Read chunk size line
            let size_line = self.read_line(reader).await?;
            let size_str = size_line.split(';').next().unwrap_or(&size_line).trim();
            let chunk_size = usize::from_str_radix(size_str, 16)
                .map_err(|_| ParseError::InvalidContentLength)?;

            if chunk_size == 0 {
                // Read trailing CRLF
                let _ = self.read_line(reader).await?;
                break;
            }

            // Read chunk data
            let chunk = self.read_exact(reader, chunk_size).await?;
            body.extend_from_slice(&chunk);

            // Read trailing CRLF
            let _ = self.read_line(reader).await?;
        }

        Ok(body)
    }

    /// Read a line (up to CRLF) from buffer/stream.
    async fn read_line<R>(&mut self, reader: &mut R) -> Result<String, ParseError>
    where
        R: KimojioAsyncRead,
    {
        let mut line = Vec::new();

        loop {
            // Check buffer for newline
            if let Some(pos) = self.buf[..self.len].iter().position(|&b| b == b'\n') {
                line.extend_from_slice(&self.buf[..pos]);
                // Remove \r if present
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                // Remove line from buffer (including \n)
                self.buf.copy_within(pos + 1..self.len, 0);
                self.len -= pos + 1;
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }

            // Add all buffer to line and read more
            line.extend_from_slice(&self.buf[..self.len]);
            self.len = 0;

            let n = self.read_more(reader).await?;
            if n == 0 {
                return Err(ParseError::ConnectionClosed);
            }
        }
    }

    /// Read exact number of bytes.
    async fn read_exact<R>(&mut self, reader: &mut R, len: usize) -> Result<Vec<u8>, ParseError>
    where
        R: KimojioAsyncRead,
    {
        let mut data = Vec::with_capacity(len);

        // Use buffer first
        let from_buf = self.len.min(len);
        data.extend_from_slice(&self.buf[..from_buf]);
        self.buf.copy_within(from_buf..self.len, 0);
        self.len -= from_buf;

        // Read remaining
        while data.len() < len {
            let remaining = len - data.len();
            let mut chunk = vec![0u8; remaining.min(8192)];
            let n = reader.try_read(&mut chunk).await?;
            if n == 0 {
                return Err(ParseError::ConnectionClosed);
            }
            data.extend_from_slice(&chunk[..n]);
        }

        Ok(data)
    }

    /// Reset the parser for reuse.
    pub fn reset(&mut self) {
        self.len = 0;
    }
}

impl Default for KimojioHttpParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for kimojio-style async read operations.
///
/// This trait abstracts over kimojio's `AsyncStreamRead` for testing
/// and flexibility. In production, this would be implemented by
/// `OwnedFdStream` or similar types.
pub trait KimojioAsyncRead {
    /// Try to read data into the buffer.
    /// Returns the number of bytes read (0 means EOF).
    fn try_read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, ParseError>>;
}

/// Trait for kimojio-style async write operations.
pub trait KimojioAsyncWrite {
    /// Write the entire buffer to the stream.
    fn write_all(&mut self, buf: &[u8]) -> impl Future<Output = Result<(), ParseError>>;

    /// Shutdown the write side of the stream.
    fn shutdown(&mut self) -> impl Future<Output = Result<(), ParseError>>;
}

/// Serialize an HTTP response to bytes.
pub fn serialize_response(response: &Response<Bytes>) -> Vec<u8> {
    let body = response.body();
    let mut buf = Vec::with_capacity(256 + body.len());

    // Status line
    buf.extend_from_slice(version_str(response.version()).as_bytes());
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(response.status().as_str().as_bytes());
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(reason_phrase(response.status()).as_bytes());
    buf.extend_from_slice(b"\r\n");

    // Headers
    for (key, value) in response.headers() {
        buf.extend_from_slice(key.as_str().as_bytes());
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(value.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }

    // Content-Length if not already set
    if !response.headers().contains_key(header::CONTENT_LENGTH) {
        buf.extend_from_slice(b"Content-Length: ");
        buf.extend_from_slice(body.len().to_string().as_bytes());
        buf.extend_from_slice(b"\r\n");
    }

    // End of headers
    buf.extend_from_slice(b"\r\n");

    // Body
    buf.extend_from_slice(body);

    buf
}

/// Write an HTTP response using kimojio's completion-based write.
pub async fn write_response<W: KimojioAsyncWrite>(
    writer: &mut W,
    response: &Response<Bytes>,
) -> Result<(), ParseError> {
    let bytes = serialize_response(response);
    writer.write_all(&bytes).await
}

/// Handle a single HTTP connection with keep-alive support.
///
/// This function handles potentially multiple HTTP requests on a single
/// connection using HTTP/1.1 keep-alive.
pub async fn handle_http_connection<R, W, F, Fut>(
    reader: &mut R,
    writer: &mut W,
    handler: F,
) -> Result<(), ParseError>
where
    R: KimojioAsyncRead,
    W: KimojioAsyncWrite,
    F: Fn(Request<Bytes>) -> Fut + Clone,
    Fut: Future<Output = Response<Bytes>>,
{
    let mut parser = KimojioHttpParser::new();

    loop {
        let request = match parser.parse_request(reader).await {
            Ok(Some(req)) => req,
            Ok(None) => {
                // Clean connection close
                return Ok(());
            }
            Err(ParseError::ConnectionClosed) => {
                return Ok(());
            }
            Err(e) => {
                // Try to send error response
                let response = Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header(header::CONNECTION, "close")
                    .body(Bytes::from(format!("Parse error: {}", e)))
                    .unwrap();
                let _ = write_response(writer, &response).await;
                return Err(e);
            }
        };

        let keep_alive = should_keep_alive(request.headers(), request.version());

        // Call handler
        let mut response = handler(request).await;

        // Set Connection header based on keep-alive
        if !keep_alive {
            response
                .headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("close"));
        }

        // Write response
        write_response(writer, &response).await?;

        if !keep_alive {
            return Ok(());
        }

        // Reset parser for next request
        parser.reset();
    }
}

/// Simple echo handler for testing - echoes the request body back.
pub async fn simple_echo_handler(req: Request<Bytes>) -> Response<Bytes> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(req.into_body())
        .unwrap()
}

/// Run a kimojio-based HTTP server with a thread-per-core architecture.
///
/// This function spawns one thread per CPU core, each running its own
/// kimojio runtime with io_uring. Each thread binds to a specific core
/// and uses SO_REUSEPORT for load balancing across cores.
///
/// # Arguments
/// * `port` - The port to listen on
/// * `handler` - An async function that handles HTTP requests
///
/// # Example
///
/// ```ignore
/// use hyper::body::Bytes;
/// use hyper::{Request, Response, StatusCode};
/// use http_body_util::Full;
///
/// async fn my_handler(_req: Request<Bytes>) -> Response<Bytes> {
///     Response::builder()
///         .status(StatusCode::OK)
///         .body(Bytes::from("Hello!"))
///         .unwrap()
/// }
///
/// run_kimojio_thread_per_core_server(8080, my_handler, false);
/// ```
pub fn run_kimojio_thread_per_core_server<F, Fut>(port: u16, handler: F, busy_poll: bool)
where
    F: Fn(Request<Bytes>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Response<Bytes>> + 'static,
{
    use kimojio::configuration::{BusyPoll, Configuration};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    let num_cores = thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);

    println!(
        "[kimojio] Starting thread-per-core HTTP server on port {} with {} cores{}",
        port,
        num_cores,
        if busy_poll { " (busy polling)" } else { "" }
    );

    let shutdown = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(num_cores);

    // Spawn worker threads for all cores
    for core_id in 0..num_cores {
        let handler = handler.clone();
        let shutdown = shutdown.clone();

        let handle = thread::Builder::new()
            .name(format!("kimojio-core-{}", core_id))
            .spawn(move || {
                // Pin thread to core for better cache locality
                if let Err(e) = dpdk_net::api::rte::thread::set_cpu_affinity(core_id) {
                    eprintln!(
                        "[kimojio] core={} Failed to set CPU affinity: {}",
                        core_id, e
                    );
                }

                // Configure busy polling if requested
                let config = if busy_poll {
                    Configuration::new().set_busy_poll(BusyPoll::Always)
                } else {
                    Configuration::new()
                };

                // Run the kimojio runtime with thread index (core_id)
                let result = kimojio::run_with_configuration(
                    core_id as u8,
                    async move { run_kimojio_accept_loop(core_id, port, handler, shutdown).await },
                    config,
                );

                match result {
                    Some(Ok(Ok(()))) => println!("[kimojio] core={} Worker stopped", core_id),
                    Some(Ok(Err(e))) => {
                        eprintln!("[kimojio] core={} Worker error: {:?}", core_id, e)
                    }
                    Some(Err(e)) => {
                        eprintln!("[kimojio] core={} Worker panicked: {:?}", core_id, e)
                    }
                    None => println!("[kimojio] core={} Worker cancelled", core_id),
                }
            })
            .expect("Failed to spawn worker thread");

        handles.push(handle);
    }

    // Wait for Ctrl+C on the main thread
    println!("[kimojio] Press Ctrl+C to stop the server");
    let shutdown_for_signal = shutdown.clone();
    ctrlc::set_handler(move || {
        println!("\n[kimojio] Received Ctrl+C, shutting down...");
        shutdown_for_signal.store(true, Ordering::SeqCst);
    })
    .expect("Failed to set Ctrl+C handler");

    // Wait for all worker threads to finish
    for handle in handles {
        let _ = handle.join();
    }

    println!("[kimojio] Server stopped");
}

/// Run the accept loop for a single kimojio core.
async fn run_kimojio_accept_loop<F, Fut>(
    core_id: usize,
    port: u16,
    handler: F,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), kimojio::Errno>
where
    F: Fn(Request<Bytes>) -> Fut + Clone + 'static,
    Fut: Future<Output = Response<Bytes>> + 'static,
{
    use kimojio::SplittableStream;
    use kimojio::operations::{self, spawn_task};

    // Create server socket with SO_REUSEPORT for thread-per-core load balancing
    let server_fd = create_server_socket_reuseport(port).await?;

    println!("[kimojio] core={} Listening on port {}", core_id, port);

    loop {
        // Check for shutdown
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        // Accept with a timeout so we can check shutdown periodically
        // For now, we'll just accept and spawn handlers
        let client_fd = operations::accept(&server_fd).await?;
        let stream = OwnedFdStream::new(client_fd);

        let handler = handler.clone();
        spawn_task(async move {
            // Split the stream into read and write halves
            let (mut reader, mut writer) = match stream.split().await {
                Ok(halves) => halves,
                Err(e) => {
                    eprintln!("[kimojio] Failed to split stream: {:?}", e);
                    return;
                }
            };

            if let Err(e) = handle_http_connection(&mut reader, &mut writer, |req| {
                let handler = handler.clone();
                async move { handler(req).await }
            })
            .await
            {
                // Connection errors are expected (client disconnect, etc.)
                if !matches!(e, ParseError::ConnectionClosed) {
                    eprintln!("[kimojio] Connection error: {}", e);
                }
            }
        });
    }

    Ok(())
}

// ============================================================================
// Kimojio-specific implementation
// ============================================================================

use kimojio::{
    AsyncStreamRead, AsyncStreamWrite, OwnedFdStream, OwnedFdStreamRead, OwnedFdStreamWrite,
};

impl KimojioAsyncRead for OwnedFdStream {
    async fn try_read(&mut self, buf: &mut [u8]) -> Result<usize, ParseError> {
        AsyncStreamRead::try_read(self, buf, None)
            .await
            .map_err(|e| ParseError::Io(format!("{:?}", e)))
    }
}

impl KimojioAsyncWrite for OwnedFdStream {
    async fn write_all(&mut self, buf: &[u8]) -> Result<(), ParseError> {
        AsyncStreamWrite::write(self, buf, None)
            .await
            .map_err(|e| ParseError::Io(format!("{:?}", e)))
    }

    async fn shutdown(&mut self) -> Result<(), ParseError> {
        AsyncStreamWrite::shutdown(self)
            .await
            .map_err(|e| ParseError::Io(format!("{:?}", e)))
    }
}

impl KimojioAsyncRead for OwnedFdStreamRead {
    async fn try_read(&mut self, buf: &mut [u8]) -> Result<usize, ParseError> {
        AsyncStreamRead::try_read(self, buf, None)
            .await
            .map_err(|e| ParseError::Io(format!("{:?}", e)))
    }
}

impl KimojioAsyncWrite for OwnedFdStreamWrite {
    async fn write_all(&mut self, buf: &[u8]) -> Result<(), ParseError> {
        AsyncStreamWrite::write(self, buf, None)
            .await
            .map_err(|e| ParseError::Io(format!("{:?}", e)))
    }

    async fn shutdown(&mut self) -> Result<(), ParseError> {
        AsyncStreamWrite::shutdown(self)
            .await
            .map_err(|e| ParseError::Io(format!("{:?}", e)))
    }
}

// ============================================================================
// Tests using in-memory buffer streams
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple in-memory reader for testing
    struct TestReader {
        data: Vec<u8>,
        pos: usize,
    }

    impl TestReader {
        fn new(data: &[u8]) -> Self {
            Self {
                data: data.to_vec(),
                pos: 0,
            }
        }
    }

    impl KimojioAsyncRead for TestReader {
        async fn try_read(&mut self, buf: &mut [u8]) -> Result<usize, ParseError> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let remaining = &self.data[self.pos..];
            let to_read = remaining.len().min(buf.len());
            buf[..to_read].copy_from_slice(&remaining[..to_read]);
            self.pos += to_read;
            Ok(to_read)
        }
    }

    /// A simple in-memory writer for testing
    struct TestWriter {
        data: Vec<u8>,
    }

    impl TestWriter {
        fn new() -> Self {
            Self { data: Vec::new() }
        }
    }

    impl KimojioAsyncWrite for TestWriter {
        async fn write_all(&mut self, buf: &[u8]) -> Result<(), ParseError> {
            self.data.extend_from_slice(buf);
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), ParseError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_parse_simple_request() {
        let request_data =
            b"GET /hello HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nHello";
        let mut reader = TestReader::new(request_data);
        let mut parser = KimojioHttpParser::new();

        let request = parser.parse_request(&mut reader).await.unwrap().unwrap();
        assert_eq!(request.method(), Method::GET);
        assert_eq!(request.uri().path(), "/hello");
        assert_eq!(request.version(), Version::HTTP_11);
        assert_eq!(request.headers().get("host").unwrap(), "localhost");
        assert_eq!(request.body().as_ref(), b"Hello");
    }

    #[tokio::test]
    async fn test_parse_chunked_request() {
        let request_data = b"POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n6\r\n World\r\n0\r\n\r\n";
        let mut reader = TestReader::new(request_data);
        let mut parser = KimojioHttpParser::new();

        let request = parser.parse_request(&mut reader).await.unwrap().unwrap();
        assert_eq!(request.method(), Method::POST);
        assert_eq!(request.body().as_ref(), b"Hello World");
    }

    #[tokio::test]
    async fn test_response_serialization() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("X-Custom", "test")
            .body(Bytes::from("Hello"))
            .unwrap();

        let bytes = serialize_response(&response);
        let text = String::from_utf8_lossy(&bytes);

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("x-custom: test\r\n"));
        assert!(text.contains("Content-Length: 5\r\n"));
        assert!(text.ends_with("\r\n\r\nHello"));
    }

    #[tokio::test]
    async fn test_handle_connection() {
        let request_data = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        let mut reader = TestReader::new(request_data);
        let mut writer = TestWriter::new();

        let result = handle_http_connection(&mut reader, &mut writer, simple_echo_handler).await;
        assert!(result.is_ok());

        let response_text = String::from_utf8_lossy(&writer.data);
        assert!(response_text.starts_with("HTTP/1.1 200 OK\r\n"));
    }

    #[test]
    fn test_keep_alive() {
        let mut headers = HeaderMap::new();
        assert!(should_keep_alive(&headers, Version::HTTP_11));
        assert!(!should_keep_alive(&headers, Version::HTTP_10));

        headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        assert!(should_keep_alive(&headers, Version::HTTP_10));

        headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
        assert!(!should_keep_alive(&headers, Version::HTTP_11));
    }
}

// ============================================================================
// Kimojio integration tests using real TCP sockets
// ============================================================================

#[cfg(test)]
mod kimojio_integration_tests {
    use super::*;
    use kimojio::{
        OwnedFdStream,
        operations::{self, AddressFamily, SocketType, spawn_task},
        socket_helpers,
    };
    use std::net::{Ipv4Addr, SocketAddrV4};

    /// Simple handler that returns "Hello, World!"
    async fn hello_handler(_req: Request<Bytes>) -> Response<Bytes> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Bytes::from("Hello, World!"))
            .unwrap()
    }

    /// Handler that echoes the request body with method info
    async fn echo_with_info(req: Request<Bytes>) -> Response<Bytes> {
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let body = req.into_body();
        let response_body = format!(
            "{} {} - Body: {}",
            method,
            path,
            String::from_utf8_lossy(&body)
        );

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Bytes::from(response_body))
            .unwrap()
    }

    #[kimojio::test]
    async fn test_kimojio_http_hello_world() {
        // Use port 0 to get an available port - but kimojio's create_server_socket takes a fixed port
        // We'll use a high port that's likely available
        let port: u16 = 19876;
        let server_fd = socket_helpers::create_server_socket(port).await.unwrap();

        // Spawn server task
        spawn_task(async move {
            let client_fd = operations::accept(&server_fd).await.unwrap();
            let mut stream = OwnedFdStream::new(client_fd);

            let mut parser = KimojioHttpParser::new();
            if let Ok(Some(_request)) = parser.parse_request(&mut stream).await {
                let response = hello_handler(Request::new(Bytes::new())).await;
                let _ = write_response(&mut stream, &response).await;
            }
        });

        // Give server time to start listening
        let _ = operations::sleep(std::time::Duration::from_millis(10)).await;

        // Create client socket and connect
        let client_fd = operations::socket(AddressFamily::INET, SocketType::STREAM, None)
            .await
            .unwrap();
        let server_addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        operations::connect(&client_fd, &server_addr.into())
            .await
            .unwrap();
        let mut client_stream = OwnedFdStream::new(client_fd);

        // Send HTTP request
        let request = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        AsyncStreamWrite::write(&mut client_stream, request, None)
            .await
            .unwrap();

        // Read response
        let mut response_buf = vec![0u8; 1024];
        let n = AsyncStreamRead::try_read(&mut client_stream, &mut response_buf, None)
            .await
            .unwrap();
        let response_text = String::from_utf8_lossy(&response_buf[..n]);

        assert!(response_text.contains("HTTP/1.1 200 OK"));
        assert!(response_text.contains("Hello, World!"));
    }

    #[kimojio::test]
    async fn test_kimojio_http_echo_post() {
        // Use a different port for this test
        let port: u16 = 19877;
        let server_fd = socket_helpers::create_server_socket(port).await.unwrap();

        // Spawn server task
        spawn_task(async move {
            let client_fd = operations::accept(&server_fd).await.unwrap();
            let mut stream = OwnedFdStream::new(client_fd);

            let mut parser = KimojioHttpParser::new();
            if let Ok(Some(request)) = parser.parse_request(&mut stream).await {
                let response = echo_with_info(request).await;
                let _ = write_response(&mut stream, &response).await;
            }
        });

        // Give server time to start listening
        let _ = operations::sleep(std::time::Duration::from_millis(10)).await;

        // Create client socket and connect
        let client_fd = operations::socket(AddressFamily::INET, SocketType::STREAM, None)
            .await
            .unwrap();
        let server_addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        operations::connect(&client_fd, &server_addr.into())
            .await
            .unwrap();
        let mut client_stream = OwnedFdStream::new(client_fd);

        // Send HTTP POST request with body
        let request = b"POST /api/data HTTP/1.1\r\nHost: localhost\r\nContent-Length: 11\r\nConnection: close\r\n\r\nHello World";
        AsyncStreamWrite::write(&mut client_stream, request, None)
            .await
            .unwrap();

        // Read response
        let mut response_buf = vec![0u8; 1024];
        let n = AsyncStreamRead::try_read(&mut client_stream, &mut response_buf, None)
            .await
            .unwrap();
        let response_text = String::from_utf8_lossy(&response_buf[..n]);

        assert!(response_text.contains("HTTP/1.1 200 OK"));
        assert!(response_text.contains("POST /api/data"));
        assert!(response_text.contains("Body: Hello World"));
    }
}

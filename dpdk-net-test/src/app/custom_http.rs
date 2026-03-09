//! A minimal HTTP/1.1 implementation for async streams.
//!
//! This module provides a simple HTTP/1.1 parser and server that works with
//! tokio's async I/O traits. It uses `httparse` for fast, zero-copy header parsing
//! and reuses standard `http` crate types with `Bytes` as the non-streaming body.
//!
//! # Features
//!
//! - HTTP/1.1 request parsing using `httparse` (same parser hyper uses)
//! - HTTP/1.1 response serialization
//! - Simple server implementation with custom handlers
//! - Support for chunked transfer encoding
//! - Connection keep-alive support
//! - Compatible with hyper's types for easy migration
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net_test::app::custom_http::SimpleHttp1Server;
//! use dpdk_net::socket::TcpListener;
//! use tokio_util::sync::CancellationToken;
//! use hyper::body::Bytes;
//! use hyper::http::{Request, Response, StatusCode};
//!
//! async fn my_handler(req: Request<Bytes>) -> Response<Bytes> {
//!     Response::builder()
//!         .status(StatusCode::OK)
//!         .header("Content-Type", "text/plain")
//!         .body(Bytes::from("Hello, World!"))
//!         .unwrap()
//! }
//!
//! async fn run_server(listener: TcpListener, cancel: CancellationToken) {
//!     let server = SimpleHttp1Server::new(listener, cancel, my_handler, 0, 8080);
//!     server.run().await;
//! }
//! ```

use std::fmt;
use std::future::Future;
use std::io;

use hyper::body::Bytes;
use hyper::header;
use hyper::http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Version,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, info};

use dpdk_net::runtime::compat_stream::AsyncTcpStream;
use dpdk_net::socket::TcpListener;
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};
use tokio_util::sync::CancellationToken;

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
    Io(io::Error),
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

impl From<io::Error> for ParseError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            ParseError::ConnectionClosed
        } else {
            ParseError::Io(e)
        }
    }
}

impl From<httparse::Error> for ParseError {
    fn from(e: httparse::Error) -> Self {
        ParseError::Parse(e)
    }
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
fn should_keep_alive(headers: &HeaderMap, version: Version) -> bool {
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

/// HTTP request parser for async streams using httparse.
pub struct HttpParser<R> {
    reader: R,
    buf: Vec<u8>,
    /// Number of valid bytes in the buffer
    len: usize,
}

impl<R: AsyncRead + Unpin> HttpParser<R> {
    /// Create a new parser wrapping an async reader.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            buf: vec![0u8; INITIAL_BUF_SIZE],
            len: 0,
        }
    }

    /// Get a mutable reference to the inner reader.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    /// Parse an HTTP request from the stream.
    ///
    /// Returns `None` if the connection was closed cleanly before any data was received.
    pub async fn parse_request(&mut self) -> Result<Option<Request<Bytes>>, ParseError> {
        // Read and parse headers
        let (method, uri, version, headers, header_len) = match self.parse_headers().await? {
            Some(parsed) => parsed,
            None => return Ok(None),
        };

        // Remove parsed headers from buffer, keeping any leftover body data
        self.buf.copy_within(header_len..self.len, 0);
        self.len -= header_len;

        // Read body based on Content-Length or Transfer-Encoding
        let body = self.read_body(&headers).await?;

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
    async fn parse_headers(
        &mut self,
    ) -> Result<Option<(Method, String, Version, HeaderMap, usize)>, ParseError> {
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
                        let n = self.read_more().await?;
                        if n == 0 {
                            return Ok(None); // Clean close before any data
                        }
                    } else {
                        // Continue reading
                        let n = self.read_more().await?;
                        if n == 0 {
                            return Err(ParseError::ConnectionClosed);
                        }
                    }
                }
            }
        }
    }

    /// Read more data into the buffer.
    async fn read_more(&mut self) -> Result<usize, ParseError> {
        // Grow buffer if needed
        if self.len == self.buf.len() {
            if self.buf.len() >= MAX_BUF_SIZE {
                return Err(ParseError::HeadersTooLarge);
            }
            self.buf.resize(self.buf.len() * 2, 0);
        }

        let n = self.reader.read(&mut self.buf[self.len..]).await?;
        self.len += n;
        Ok(n)
    }

    /// Read the request body.
    async fn read_body(&mut self, headers: &HeaderMap) -> Result<Vec<u8>, ParseError> {
        if is_chunked(headers) {
            return self.read_chunked_body().await;
        }

        if let Some(content_length) = get_content_length(headers) {
            return self.read_fixed_body(content_length).await;
        }

        Ok(Vec::new())
    }

    /// Read a fixed-size body.
    async fn read_fixed_body(&mut self, length: usize) -> Result<Vec<u8>, ParseError> {
        let mut body = Vec::with_capacity(length);

        // Use any data already in buffer
        let from_buf = self.len.min(length);
        body.extend_from_slice(&self.buf[..from_buf]);
        self.buf.copy_within(from_buf..self.len, 0);
        self.len -= from_buf;

        // Read remaining directly
        if body.len() < length {
            body.resize(length, 0);
            self.reader.read_exact(&mut body[from_buf..]).await?;
        }

        Ok(body)
    }

    /// Read a chunked transfer-encoded body.
    async fn read_chunked_body(&mut self) -> Result<Vec<u8>, ParseError> {
        let mut body = Vec::new();

        loop {
            // Read chunk size line
            let size_line = self.read_line().await?;
            let size_str = size_line.split(';').next().unwrap_or(&size_line).trim();
            let chunk_size = usize::from_str_radix(size_str, 16)
                .map_err(|_| ParseError::InvalidContentLength)?;

            if chunk_size == 0 {
                // Read trailing CRLF
                let _ = self.read_line().await?;
                break;
            }

            // Read chunk data
            let chunk = self.read_exact(chunk_size).await?;
            body.extend_from_slice(&chunk);

            // Read trailing CRLF
            let _ = self.read_line().await?;
        }

        Ok(body)
    }

    /// Read a line (up to CRLF) from buffer/stream.
    async fn read_line(&mut self) -> Result<String, ParseError> {
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

            let n = self.read_more().await?;
            if n == 0 {
                return Err(ParseError::ConnectionClosed);
            }
        }
    }

    /// Read exact number of bytes.
    async fn read_exact(&mut self, len: usize) -> Result<Vec<u8>, ParseError> {
        let mut data = Vec::with_capacity(len);

        // Use buffer first
        let from_buf = self.len.min(len);
        data.extend_from_slice(&self.buf[..from_buf]);
        self.buf.copy_within(from_buf..self.len, 0);
        self.len -= from_buf;

        // Read remaining
        if data.len() < len {
            data.resize(len, 0);
            self.reader.read_exact(&mut data[from_buf..]).await?;
        }

        Ok(data)
    }
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

/// Write an HTTP response to an async writer.
pub async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: &Response<Bytes>,
) -> io::Result<()> {
    let bytes = serialize_response(response);
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Simple HTTP/1.1 server with custom handler.
///
/// This is a lightweight alternative to the hyper-based `Http1Server`.
/// Uses `httparse` for parsing and standard `http` crate types with `Bytes` as the body.
pub struct SimpleHttp1Server<F> {
    listener: TcpListener,
    cancel: CancellationToken,
    handler: F,
    queue_id: usize,
    port: u16,
}

impl<F, Fut> SimpleHttp1Server<F>
where
    F: Fn(Request<Bytes>) -> Fut + Clone + 'static,
    Fut: Future<Output = Response<Bytes>> + 'static,
{
    /// Create a new simple HTTP/1.1 server.
    pub fn new(
        listener: TcpListener,
        cancel: CancellationToken,
        handler: F,
        queue_id: usize,
        port: u16,
    ) -> Self {
        Self {
            listener,
            cancel,
            handler,
            queue_id,
            port,
        }
    }

    /// Run the server until cancellation.
    pub async fn run(mut self) {
        info!(
            queue_id = self.queue_id,
            port = self.port,
            "Simple HTTP/1.1 Server listening"
        );

        let mut conn_id = 0u64;

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    break;
                }
                result = self.listener.accept() => {
                    match result {
                        Ok(stream) => {
                            let id = conn_id;
                            conn_id += 1;
                            let queue_id = self.queue_id;
                            debug!(queue_id, conn_id = id, "HTTP/1.1 connection accepted");

                            let io = AsyncTcpStream::new(stream).compat();
                            let handler = self.handler.clone();

                            tokio::task::spawn_local(async move {
                                if let Err(e) = handle_connection(io, handler, queue_id, id).await {
                                    debug!(queue_id, conn_id = id, error = %e, "HTTP/1.1 connection error");
                                } else {
                                    debug!(queue_id, conn_id = id, "HTTP/1.1 connection closed");
                                }
                            });
                        }
                        Err(e) => {
                            error!(queue_id = self.queue_id, error = ?e, "HTTP/1.1 accept failed");
                        }
                    }
                }
            }
        }

        info!(
            queue_id = self.queue_id,
            last_conn = conn_id,
            "Simple HTTP/1.1 server shutting down"
        );
    }
}

/// Handle a single HTTP connection (potentially multiple requests with keep-alive).
async fn handle_connection<F, Fut>(
    io: Compat<AsyncTcpStream>,
    handler: F,
    queue_id: usize,
    conn_id: u64,
) -> Result<(), ParseError>
where
    F: Fn(Request<Bytes>) -> Fut + Clone + 'static,
    Fut: Future<Output = Response<Bytes>> + 'static,
{
    let mut parser = HttpParser::new(io);

    loop {
        let request = match parser.parse_request().await {
            Ok(Some(req)) => req,
            Ok(None) => {
                debug!(queue_id, conn_id, "Client closed connection");
                return Ok(());
            }
            Err(ParseError::ConnectionClosed) => {
                debug!(queue_id, conn_id, "Connection closed");
                return Ok(());
            }
            Err(e) => {
                debug!(queue_id, conn_id, error = %e, "Parse error");
                let response = Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header(header::CONNECTION, "close")
                    .body(Bytes::from(format!("Parse error: {}", e)))
                    .unwrap();
                let _ = write_response(parser.get_mut(), &response).await;
                return Err(e);
            }
        };

        let keep_alive = should_keep_alive(request.headers(), request.version());

        debug!(
            queue_id,
            conn_id,
            method = %request.method(),
            uri = %request.uri(),
            version = ?request.version(),
            body_len = request.body().len(),
            keep_alive,
            "HTTP request received"
        );

        // Call handler
        let mut response = handler(request).await;

        // Set Connection header based on keep-alive
        if !keep_alive {
            response
                .headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("close"));
        }

        // Write response
        if let Err(e) = write_response(parser.get_mut(), &response).await {
            debug!(queue_id, conn_id, error = %e, "Failed to write response");
            return Err(ParseError::Io(e));
        }

        if !keep_alive {
            return Ok(());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_serialization() {
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

    #[tokio::test]
    async fn test_parse_simple_request() {
        let request_data =
            b"GET /hello HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nHello";
        let mut parser = HttpParser::new(&request_data[..]);

        let request = parser.parse_request().await.unwrap().unwrap();
        assert_eq!(request.method(), Method::GET);
        assert_eq!(request.uri().path(), "/hello");
        assert_eq!(request.version(), Version::HTTP_11);
        assert_eq!(request.headers().get("host").unwrap(), "localhost");
        assert_eq!(request.body().as_ref(), b"Hello");
    }

    #[tokio::test]
    async fn test_parse_chunked_request() {
        let request_data = b"POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n6\r\n World\r\n0\r\n\r\n";
        let mut parser = HttpParser::new(&request_data[..]);

        let request = parser.parse_request().await.unwrap().unwrap();
        assert_eq!(request.method(), Method::POST);
        assert_eq!(request.body().as_ref(), b"Hello World");
    }

    #[tokio::test]
    async fn test_parse_no_body() {
        let request_data = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut parser = HttpParser::new(&request_data[..]);

        let request = parser.parse_request().await.unwrap().unwrap();
        assert_eq!(request.method(), Method::GET);
        assert!(request.body().is_empty());
    }
}

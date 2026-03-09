//! Reusable async HTTP server components for DPDK + smoltcp + hyper.
//!
//! This module provides generic HTTP servers that can run with custom handlers:
//! - `Http1Server` - HTTP/1.1 only
//! - `Http2Server` - HTTP/2 only (cleartext h2c)
//! - `HttpAutoServer` - Auto-detects HTTP/1.1 or HTTP/2
//!
//! Also provides a default `echo_service` handler for testing.
//!
//! # Example
//!
//! ```no_run
//! use dpdk_net_test::app::http_server::{HttpAutoServer, Http1Server, echo_service};
//! use dpdk_net::socket::TcpListener;
//! use tokio_util::sync::CancellationToken;
//!
//! // Using the default echo handler
//! async fn run_echo(listener: TcpListener, cancel: CancellationToken) {
//!     let server = Http1Server::new(listener, cancel, echo_service, 0, 8080);
//!     server.run().await;
//! }
//!
//! // Using a custom handler
//! use http_body_util::Full;
//! use hyper::body::Bytes;
//! use hyper::{Request, Response, StatusCode};
//!
//! async fn my_handler(req: Request<Bytes>) -> Result<Response<Full<Bytes>>, hyper::Error> {
//!     Ok(Response::builder()
//!         .status(StatusCode::OK)
//!         .body(Full::new(Bytes::from("Hello!")))
//!         .unwrap())
//! }
//!
//! async fn run_custom(listener: TcpListener, cancel: CancellationToken) {
//!     let server = Http1Server::new(listener, cancel, my_handler, 0, 8080);
//!     server.run().await;
//! }
//! ```

use std::future::Future;

use dpdk_net::runtime::compat_stream::AsyncTcpStream;
use dpdk_net::socket::TcpListener;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{debug, error, info};

use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1 as server_http1;
use hyper::server::conn::http2 as server_http2;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as AutoBuilder;

use tokio_util::sync::CancellationToken;

/// A local executor for hyper that uses spawn_local instead of spawn.
///
/// Since our TcpStream is !Send (uses Rc), we need an executor that
/// spawns tasks on the local thread.
#[derive(Clone, Copy)]
pub struct LocalExecutor;

impl<F> hyper::rt::Executor<F> for LocalExecutor
where
    F: std::future::Future + 'static,
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        tokio::task::spawn_local(fut);
    }
}

/// HTTP echo service handler - echoes the request body back.
///
/// This function handles HTTP requests by echoing the request body
/// back in the response. Works with both HTTP/1.1 and HTTP/2.
pub async fn echo_service(req: Request<Bytes>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();
    debug!(version = ?version, %method, %uri, "HTTP request received");

    // Get the request body (already collected)
    let body_bytes = req.into_body();
    debug!(bytes = body_bytes.len(), "HTTP body received");

    // Echo it back
    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .body(Full::new(body_bytes))
        .unwrap();

    Ok(response)
}

/// Wrap a handler that takes `Request<Bytes>` to work with hyper's `Request<Incoming>`.
///
/// This adapter collects the streaming body into `Bytes` before calling the handler,
/// allowing handlers to be written with non-streaming body types.
#[allow(clippy::type_complexity)]
fn with_collected_body<F, Fut>(
    handler: F,
) -> impl Fn(
    Request<Incoming>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Response<Full<Bytes>>, hyper::Error>>>,
> + Clone
+ 'static
where
    F: Fn(Request<Bytes>) -> Fut + Clone + 'static,
    Fut: std::future::Future<Output = Result<Response<Full<Bytes>>, hyper::Error>> + 'static,
{
    move |req: Request<Incoming>| {
        let handler = handler.clone();
        Box::pin(async move {
            // Split request into parts and body
            let (parts, body) = req.into_parts();
            // Collect the body
            let body_bytes = body.collect().await?.to_bytes();
            // Reconstruct with Bytes body
            let req = Request::from_parts(parts, body_bytes);
            handler(req).await
        })
    }
}

/// HTTP/1+2 Auto Server with custom handler.
///
/// Accepts TCP connections and serves both HTTP/1.1 and HTTP/2 (cleartext h2c)
/// using hyper-util's auto builder.
pub struct HttpAutoServer<F> {
    listener: TcpListener,
    cancel: CancellationToken,
    handler: F,
    queue_id: usize,
    port: u16,
}

impl<F, Fut> HttpAutoServer<F>
where
    F: Fn(Request<Bytes>) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<Response<Full<Bytes>>, hyper::Error>> + 'static,
{
    /// Create a new HTTP auto server with a custom handler.
    ///
    /// # Arguments
    /// * `listener` - The TCP listener to accept connections on
    /// * `cancel` - Cancellation token for graceful shutdown
    /// * `handler` - The request handler function (receives collected body as Bytes)
    /// * `queue_id` - Queue identifier for logging
    /// * `port` - Port number for logging
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
    ///
    /// This accepts TCP connections in a loop and spawns an HTTP handler
    /// for each connection. The handler automatically detects whether
    /// the client is using HTTP/1.1 or HTTP/2 and responds accordingly.
    pub async fn run(mut self) {
        info!(
            queue_id = self.queue_id,
            port = self.port,
            "HTTP/1+2 Auto Server listening"
        );

        let wrapped_handler = with_collected_body(self.handler);
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
                            debug!(queue_id, conn_id = id, "HTTP connection accepted");

                            let io = TokioIo::new(AsyncTcpStream::new(stream).compat());
                            let handler = wrapped_handler.clone();

                            tokio::task::spawn_local(async move {
                                let result = AutoBuilder::new(LocalExecutor)
                                    .serve_connection(io, service_fn(handler))
                                    .await;

                                match result {
                                    Ok(()) => debug!(queue_id, conn_id = id, "HTTP connection closed"),
                                    Err(e) => debug!(queue_id, conn_id = id, error = %e, "HTTP connection error"),
                                }
                            });
                        }
                        Err(e) => {
                            error!(queue_id = self.queue_id, error = ?e, "HTTP accept failed");
                        }
                    }
                }
            }
        }

        info!(queue_id = self.queue_id, "HTTP server shutting down");
    }
}

/// HTTP/1.1 Server with custom handler.
///
/// Accepts TCP connections and serves HTTP/1.1 only.
pub struct Http1Server<F> {
    listener: TcpListener,
    cancel: CancellationToken,
    handler: F,
    queue_id: usize,
    port: u16,
}

impl<F, Fut> Http1Server<F>
where
    F: Fn(Request<Bytes>) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<Response<Full<Bytes>>, hyper::Error>> + 'static,
{
    /// Create a new HTTP/1.1 server with a custom handler.
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
            "HTTP/1.1 Server listening"
        );

        let wrapped_handler = with_collected_body(self.handler);
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

                            let io = TokioIo::new(AsyncTcpStream::new(stream).compat());
                            let handler = wrapped_handler.clone();

                            tokio::task::spawn_local(async move {
                                let result = server_http1::Builder::new()
                                    .serve_connection(io, service_fn(handler))
                                    .await;

                                match result {
                                    Ok(()) => debug!(queue_id, conn_id = id, "HTTP/1.1 connection closed"),
                                    Err(e) => debug!(queue_id, conn_id = id, error = %e, "HTTP/1.1 connection error"),
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
            "HTTP/1.1 server shutting down"
        );
    }
}

/// HTTP/2 Server with custom handler (cleartext h2c).
///
/// Accepts TCP connections and serves HTTP/2 only.
pub struct Http2Server<F> {
    listener: TcpListener,
    cancel: CancellationToken,
    handler: F,
    queue_id: usize,
    port: u16,
}

impl<F, Fut> Http2Server<F>
where
    F: Fn(Request<Bytes>) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<Response<Full<Bytes>>, hyper::Error>> + 'static,
{
    /// Create a new HTTP/2 server with a custom handler.
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
            "HTTP/2 Server listening"
        );

        let wrapped_handler = with_collected_body(self.handler);
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
                            debug!(queue_id, conn_id = id, "HTTP/2 connection accepted");

                            let io = TokioIo::new(AsyncTcpStream::new(stream).compat());
                            let handler = wrapped_handler.clone();

                            tokio::task::spawn_local(async move {
                                let result = server_http2::Builder::new(LocalExecutor)
                                    .serve_connection(io, service_fn(handler))
                                    .await;

                                match result {
                                    Ok(()) => debug!(queue_id, conn_id = id, "HTTP/2 connection closed"),
                                    Err(e) => debug!(queue_id, conn_id = id, error = %e, "HTTP/2 connection error"),
                                }
                            });
                        }
                        Err(e) => {
                            error!(queue_id = self.queue_id, error = ?e, "HTTP/2 accept failed");
                        }
                    }
                }
            }
        }

        info!(queue_id = self.queue_id, "HTTP/2 server shutting down");
    }
}

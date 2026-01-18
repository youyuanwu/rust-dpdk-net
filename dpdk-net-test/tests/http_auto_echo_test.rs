//! HTTP/1+2 Auto Echo Test with Hyper
//!
//! This test creates an HTTP server that handles both HTTP/1.1 and HTTP/2
//! using hyper-util's auto builder. Clients send requests using different
//! protocol versions to verify the server handles both correctly.
//!
//! Note: HTTP/2 uses cleartext (h2c), not TLS.

use dpdk_net::BoxError;
use dpdk_net::tcp::async_net::TokioTcpStream;
use dpdk_net::tcp::{Reactor, ReactorHandle, TcpListener, TcpStream};

use dpdk_net_test::dpdk_test::DpdkTestContextBuilder;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::client::conn::http1 as client_http1;
use hyper::client::conn::http2 as client_http2;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as AutoBuilder;

use smoltcp::iface::{Config, Interface};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};

use std::future::Future;
use tokio::runtime::Builder;

const SERVER_PORT: u16 = 8080;
const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);

/// HTTP echo service handler - echoes the request body back
async fn echo_service(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();
    println!("Server: {:?} {} {}", version, method, uri);

    // Collect the request body
    let body_bytes = req.collect().await?.to_bytes();
    println!("Server: received {} bytes", body_bytes.len());

    // Echo it back
    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .body(Full::new(body_bytes))
        .unwrap();

    Ok(response)
}

/// A local executor for hyper that uses spawn_local instead of spawn.
///
/// Since our TcpStream is !Send (uses Rc), we need an executor that
/// spawns tasks on the local thread.
#[derive(Clone, Copy)]
struct LocalExecutor;

impl<F> hyper::rt::Executor<F> for LocalExecutor
where
    F: std::future::Future + 'static,
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        tokio::task::spawn_local(fut);
    }
}

/// Run the HTTP/1+2 auto server: accept connections and serve both protocols
async fn run_auto_server<F: Future<Output = ()>>(mut listener: TcpListener, shutdown: F) {
    println!(
        "HTTP/1+2 Auto Server: listening on {}:{}",
        SERVER_IP, SERVER_PORT
    );

    tokio::pin!(shutdown);

    let mut conn_id = 0usize;
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                println!("HTTP/1+2 Auto Server: shutdown signal received");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok(stream) => {
                        let id = conn_id;
                        conn_id += 1;
                        println!("HTTP/1+2 Auto Server: accepted connection {}", id);

                        // Wrap in TokioTcpStream, then TokioIo for hyper compatibility
                        let io = TokioIo::new(TokioTcpStream::new(stream));

                        // Spawn auto HTTP connection handler (handles both HTTP/1 and HTTP/2)
                        tokio::task::spawn_local(async move {
                            let result = AutoBuilder::new(LocalExecutor)
                                .serve_connection(io, service_fn(echo_service))
                                .await;

                            match result {
                                Ok(()) => println!("HTTP/1+2 Auto Server {}: connection closed", id),
                                Err(e) => eprintln!("HTTP/1+2 Auto Server {}: error: {}", id, e),
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("HTTP/1+2 Auto Server: accept failed: {:?}", e);
                        break;
                    }
                }
            }
        }
    }
}

/// Protocol version for client
#[derive(Clone, Copy, Debug)]
enum HttpVersion {
    Http1,
    Http2,
}

/// Run a single HTTP client with specified protocol version
async fn run_http_client(
    handle: &ReactorHandle,
    client_id: usize,
    local_port: u16,
    version: HttpVersion,
) -> Result<(), BoxError> {
    // Connect to server
    let stream = TcpStream::connect(
        handle,
        IpAddress::Ipv4(SERVER_IP),
        SERVER_PORT,
        local_port,
        16384,
        16384,
    )
    .map_err(|e| format!("Client {}: connect failed: {:?}", client_id, e))?;

    println!("HTTP Client {} ({:?}): connecting...", client_id, version);

    // Wait for TCP connection
    stream
        .wait_connected()
        .await
        .map_err(|_| format!("Client {}: TCP connection failed", client_id))?;

    println!("HTTP Client {} ({:?}): TCP connected", client_id, version);

    // Wrap for hyper: TokioTcpStream -> TokioIo
    let io = TokioIo::new(TokioTcpStream::new(stream));

    // Build request body
    let body_text = format!("Hello from {:?} client {}!", version, client_id);

    match version {
        HttpVersion::Http1 => {
            // HTTP/1.1 handshake
            let (mut sender, conn) = client_http1::handshake(io)
                .await
                .map_err(|e| format!("Client {}: HTTP/1 handshake failed: {}", client_id, e))?;

            println!("HTTP Client {} (HTTP/1): handshake complete", client_id);

            // Spawn connection driver
            tokio::task::spawn_local(async move {
                if let Err(e) = conn.await {
                    eprintln!("HTTP/1 Client connection error: {}", e);
                }
            });

            // Build and send request
            let request = Request::builder()
                .method("POST")
                .uri("/echo")
                .header("Host", format!("{}:{}", SERVER_IP, SERVER_PORT))
                .header("Content-Type", "text/plain")
                .body(Full::new(Bytes::from(body_text.clone())))
                .map_err(|e| format!("Client {}: request build failed: {}", client_id, e))?;

            println!("HTTP Client {} (HTTP/1): sending POST /echo", client_id);

            let response = sender
                .send_request(request)
                .await
                .map_err(|e| format!("Client {}: request failed: {}", client_id, e))?;

            verify_response(client_id, version, response, &body_text).await?;
        }
        HttpVersion::Http2 => {
            // HTTP/2 handshake
            let (mut sender, conn) = client_http2::handshake(LocalExecutor, io)
                .await
                .map_err(|e| format!("Client {}: HTTP/2 handshake failed: {}", client_id, e))?;

            println!("HTTP Client {} (HTTP/2): handshake complete", client_id);

            // Spawn connection driver
            tokio::task::spawn_local(async move {
                if let Err(e) = conn.await {
                    eprintln!("HTTP/2 Client connection error: {}", e);
                }
            });

            // Build and send request
            let request = Request::builder()
                .method("POST")
                .uri(format!("http://{}:{}/echo", SERVER_IP, SERVER_PORT))
                .header("Content-Type", "text/plain")
                .body(Full::new(Bytes::from(body_text.clone())))
                .map_err(|e| format!("Client {}: request build failed: {}", client_id, e))?;

            println!("HTTP Client {} (HTTP/2): sending POST /echo", client_id);

            let response = sender
                .send_request(request)
                .await
                .map_err(|e| format!("Client {}: request failed: {}", client_id, e))?;

            verify_response(client_id, version, response, &body_text).await?;
        }
    }

    Ok(())
}

/// Verify response matches expected body
async fn verify_response(
    client_id: usize,
    version: HttpVersion,
    response: Response<Incoming>,
    expected_body: &str,
) -> Result<(), BoxError> {
    println!(
        "HTTP Client {} ({:?}): response status: {}",
        client_id,
        version,
        response.status()
    );

    // Read response body
    let body_bytes = response
        .collect()
        .await
        .map_err(|e| format!("Client {}: body read failed: {}", client_id, e))?
        .to_bytes();

    let response_text = String::from_utf8_lossy(&body_bytes);

    // Verify echo
    if response_text != expected_body {
        return Err(format!(
            "Client {} ({:?}): MISMATCH! expected '{}', got '{}'",
            client_id, version, expected_body, response_text
        )
        .into());
    }

    println!("HTTP Client {} ({:?}): echo verified ✓", client_id, version);
    Ok(())
}

/// Run the HTTP/1+2 test with mixed protocol clients
async fn run_auto_test(
    handle: ReactorHandle,
    listener: TcpListener,
    num_clients_per_version: usize,
) -> Result<(), BoxError> {
    let total_clients = num_clients_per_version * 2;
    println!(
        "\n--- Running HTTP/1+2 Auto test with {} clients ({} HTTP/1, {} HTTP/2) ---\n",
        total_clients, num_clients_per_version, num_clients_per_version
    );

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn auto server
    let server_handle = tokio::task::spawn_local(run_auto_server(listener, async {
        let _ = shutdown_rx.await;
    }));

    // Spawn client tasks - alternating HTTP/1 and HTTP/2
    let mut client_handles = Vec::with_capacity(total_clients);
    for i in 0..num_clients_per_version {
        // HTTP/1 client
        {
            let local_port = 49152 + (i * 2) as u16;
            let handle_clone = handle.clone();
            let client_handle = tokio::task::spawn_local(async move {
                run_http_client(&handle_clone, i * 2, local_port, HttpVersion::Http1).await
            });
            client_handles.push(client_handle);
        }

        // HTTP/2 client
        {
            let local_port = 49153 + (i * 2) as u16;
            let handle_clone = handle.clone();
            let client_handle = tokio::task::spawn_local(async move {
                run_http_client(&handle_clone, i * 2 + 1, local_port, HttpVersion::Http2).await
            });
            client_handles.push(client_handle);
        }
    }

    // Wait for all clients
    let mut errors: Vec<BoxError> = Vec::new();
    for (i, handle) in client_handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e),
            Err(e) => errors.push(format!("Client {} panicked: {:?}", i, e).into()),
        }
    }

    // Signal server to shutdown
    let _ = shutdown_tx.send(());

    // Wait for server
    match server_handle.await {
        Ok(()) => {}
        Err(e) => errors.push(format!("Server task panicked: {:?}", e).into()),
    }

    if !errors.is_empty() {
        for e in &errors {
            eprintln!("Error: {}", e);
        }
        return Err(format!("{} errors occurred", errors.len()).into());
    }

    println!(
        "\n✓ All {} clients verified ({} HTTP/1, {} HTTP/2)!",
        total_clients, num_clients_per_version, num_clients_per_version
    );
    Ok(())
}

#[test]
fn test_http_auto_echo() {
    const NUM_CLIENTS_PER_VERSION: usize = 3;

    println!("\n=== HTTP/1+2 Auto Echo Test ===\n");

    // Create DPDK test context using the shared harness
    let (_ctx, mut device) = DpdkTestContextBuilder::new()
        .vdev("net_ring0")
        .mempool_name("http_auto_test_pool")
        .build()
        .expect("Failed to create DPDK test context");

    println!("DPDK context created successfully");

    // Configure smoltcp interface
    let mac_addr = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    let config = Config::new(mac_addr.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());

    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(IpCidr::new(IpAddress::Ipv4(SERVER_IP), 24))
            .unwrap();
    });

    // Create tokio runtime
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        // Create reactor
        let reactor = Reactor::new(device, iface);
        let handle = reactor.handle();

        // Spawn reactor
        tokio::task::spawn_local(async move {
            reactor.run().await;
        });

        // Create listener with larger buffers
        let total_clients = NUM_CLIENTS_PER_VERSION * 2;
        let listener =
            TcpListener::bind_with_backlog(&handle, SERVER_PORT, 16384, 16384, total_clients + 1)
                .expect("Failed to bind listener");

        // Run test
        let result = run_auto_test(handle, listener, NUM_CLIENTS_PER_VERSION).await;

        match result {
            Ok(()) => {
                println!("\n--- Test Result ---");
                println!(
                    "\n✓ HTTP/1+2 Auto Echo Test PASSED ({} HTTP/1 + {} HTTP/2 clients served)!\n",
                    NUM_CLIENTS_PER_VERSION, NUM_CLIENTS_PER_VERSION
                );
            }
            Err(e) => {
                panic!("Test failed: {}", e);
            }
        }
    });
}

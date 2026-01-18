//! HTTP/2 Echo Test with Hyper
//!
//! This test creates an HTTP/2 server and client using DPDK with smoltcp,
//! wrapped in TokioTcpStream for hyper compatibility.
//!
//! Note: This uses HTTP/2 over cleartext (h2c), not TLS.
//! The server echoes the request body back in the response.

use dpdk_net::BoxError;
use dpdk_net::tcp::async_net::TokioTcpStream;
use dpdk_net::tcp::{Reactor, ReactorHandle, TcpListener, TcpStream};

use dpdk_net_test::dpdk_test::DpdkTestContextBuilder;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::client::conn::http2 as client_http2;
use hyper::server::conn::http2 as server_http2;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;

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
    println!("Server: {} {}", method, uri);

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

/// Run the HTTP/2 server: accept connections and serve
async fn run_http2_server<F: Future<Output = ()>>(mut listener: TcpListener, shutdown: F) {
    println!("HTTP/2 Server: listening on {}:{}", SERVER_IP, SERVER_PORT);

    tokio::pin!(shutdown);

    let mut conn_id = 0usize;
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                println!("HTTP/2 Server: shutdown signal received");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok(stream) => {
                        let id = conn_id;
                        conn_id += 1;
                        println!("HTTP/2 Server: accepted connection {}", id);

                        // Wrap in TokioTcpStream, then TokioIo for hyper compatibility
                        let io = TokioIo::new(TokioTcpStream::new(stream));

                        // Spawn HTTP/2 connection handler
                        tokio::task::spawn_local(async move {
                            let result = server_http2::Builder::new(LocalExecutor)
                                .serve_connection(io, service_fn(echo_service))
                                .await;

                            match result {
                                Ok(()) => println!("HTTP/2 Server {}: connection closed", id),
                                Err(e) => eprintln!("HTTP/2 Server {}: error: {}", id, e),
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("HTTP/2 Server: accept failed: {:?}", e);
                        break;
                    }
                }
            }
        }
    }
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

/// Run a single HTTP/2 client: connect, send POST request, verify response
async fn run_http2_client(
    handle: &ReactorHandle,
    client_id: usize,
    local_port: u16,
) -> Result<(), BoxError> {
    // Connect to server
    let stream = TcpStream::connect(
        handle,
        IpAddress::Ipv4(SERVER_IP),
        SERVER_PORT,
        local_port,
        16384, // Larger buffers for HTTP/2 framing
        16384,
    )
    .map_err(|e| format!("Client {}: connect failed: {:?}", client_id, e))?;

    println!("HTTP/2 Client {}: connecting...", client_id);

    // Wait for TCP connection
    stream
        .wait_connected()
        .await
        .map_err(|_| format!("Client {}: TCP connection failed", client_id))?;

    println!("HTTP/2 Client {}: TCP connected", client_id);

    // Wrap for hyper: TokioTcpStream -> TokioIo
    let io = TokioIo::new(TokioTcpStream::new(stream));

    // Create HTTP/2 connection with local executor
    let (mut sender, conn) = client_http2::handshake(LocalExecutor, io)
        .await
        .map_err(|e| format!("Client {}: HTTP/2 handshake failed: {}", client_id, e))?;

    println!("HTTP/2 Client {}: HTTP/2 handshake complete", client_id);

    // Spawn connection driver
    tokio::task::spawn_local(async move {
        if let Err(e) = conn.await {
            eprintln!("HTTP/2 Client connection error: {}", e);
        }
    });

    // Build request with body
    let body_text = format!("Hello from HTTP/2 client {}!", client_id);
    let request = Request::builder()
        .method("POST")
        .uri(format!("http://{}:{}/echo", SERVER_IP, SERVER_PORT))
        .header("Content-Type", "text/plain")
        .body(Full::new(Bytes::from(body_text.clone())))
        .map_err(|e| format!("Client {}: request build failed: {}", client_id, e))?;

    println!("HTTP/2 Client {}: sending POST /echo", client_id);

    // Send request and get response
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| format!("Client {}: request failed: {}", client_id, e))?;

    println!(
        "HTTP/2 Client {}: response status: {}",
        client_id,
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
    if response_text != body_text {
        return Err(format!(
            "Client {}: MISMATCH! expected '{}', got '{}'",
            client_id, body_text, response_text
        )
        .into());
    }

    println!("HTTP/2 Client {}: echo verified ✓", client_id);
    Ok(())
}

/// Run the HTTP/2 test with multiple clients
async fn run_http2_test(
    handle: ReactorHandle,
    listener: TcpListener,
    num_clients: usize,
) -> Result<(), BoxError> {
    println!(
        "\n--- Running HTTP/2 test with {} clients ---\n",
        num_clients
    );

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn HTTP/2 server
    let server_handle = tokio::task::spawn_local(run_http2_server(listener, async {
        let _ = shutdown_rx.await;
    }));

    // Spawn client tasks
    let mut client_handles = Vec::with_capacity(num_clients);
    for i in 0..num_clients {
        let local_port = 49152 + i as u16;
        let handle_clone = handle.clone();

        let client_handle = tokio::task::spawn_local(async move {
            run_http2_client(&handle_clone, i, local_port).await
        });
        client_handles.push(client_handle);
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

    println!("\n✓ All {} HTTP/2 clients verified!", num_clients);
    Ok(())
}

#[test]
fn test_http2_echo() {
    const NUM_CLIENTS: usize = 3;

    println!("\n=== HTTP/2 Echo Test ===\n");

    // Create DPDK test context using the shared harness
    let (_ctx, mut device) = DpdkTestContextBuilder::new()
        .vdev("net_ring0")
        .mempool_name("http2_test_pool")
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

        // Create listener with larger buffers for HTTP/2
        let listener =
            TcpListener::bind_with_backlog(&handle, SERVER_PORT, 16384, 16384, NUM_CLIENTS + 1)
                .expect("Failed to bind listener");

        // Run test
        let result = run_http2_test(handle, listener, NUM_CLIENTS).await;

        match result {
            Ok(()) => {
                println!("\n--- Test Result ---");
                println!(
                    "\n✓ HTTP/2 Echo Test PASSED ({} clients served)!\n",
                    NUM_CLIENTS
                );
            }
            Err(e) => {
                panic!("Test failed: {}", e);
            }
        }
    });
}

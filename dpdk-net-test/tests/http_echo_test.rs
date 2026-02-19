//! HTTP/1.1 Echo Test with Hyper
//!
//! This test creates an HTTP/1.1 server and client using DPDK with smoltcp,
//! wrapped in TokioTcpStream for hyper compatibility.
//!
//! The server echoes the request body back in the response.

use dpdk_net::BoxError;
use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::runtime::ReactorHandle;
use dpdk_net::runtime::tokio_compat::TokioTcpStream;
use dpdk_net::socket::{TcpListener, TcpStream};

use dpdk_net_axum::{DpdkApp, WorkerContext};
use dpdk_net_test::app::http_server::{Http1Server, echo_service};

use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Bytes;
use hyper::client::conn::http1 as client_http1;
use hyper_util::rt::TokioIo;

use smoltcp::wire::{IpAddress, Ipv4Address};

use serial_test::serial;
use tokio_util::sync::CancellationToken;

const SERVER_PORT: u16 = 8080;
const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);

// Using Http1Server with echo_service from dpdk_net_test::app::http_server

/// Run a single HTTP client: connect, send POST request, verify response
async fn run_http_client(
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
        8192,
        8192,
    )
    .map_err(|e| format!("Client {}: connect failed: {:?}", client_id, e))?;

    println!("HTTP Client {}: connecting...", client_id);

    // Wait for TCP connection
    stream
        .wait_connected()
        .await
        .map_err(|_| format!("Client {}: TCP connection failed", client_id))?;

    println!("HTTP Client {}: TCP connected", client_id);

    // Wrap for hyper: TokioTcpStream -> TokioIo
    let io = TokioIo::new(TokioTcpStream::new(stream));

    // Create HTTP/1.1 connection
    let (mut sender, conn) = client_http1::handshake(io)
        .await
        .map_err(|e| format!("Client {}: HTTP handshake failed: {}", client_id, e))?;

    println!("HTTP Client {}: HTTP handshake complete", client_id);

    // Spawn connection driver
    tokio::task::spawn_local(async move {
        if let Err(e) = conn.await {
            eprintln!("HTTP Client connection error: {}", e);
        }
    });

    // Build request with body
    let body_text = format!("Hello from HTTP client {}!", client_id);
    let request = Request::builder()
        .method("POST")
        .uri("/echo")
        .header("Host", format!("{}:{}", SERVER_IP, SERVER_PORT))
        .header("Content-Type", "text/plain")
        .body(Full::new(Bytes::from(body_text.clone())))
        .map_err(|e| format!("Client {}: request build failed: {}", client_id, e))?;

    println!("HTTP Client {}: sending POST /echo", client_id);

    // Send request and get response
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| format!("Client {}: request failed: {}", client_id, e))?;

    println!(
        "HTTP Client {}: response status: {}",
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

    println!("HTTP Client {}: echo verified ✓", client_id);
    Ok(())
}

/// Run the HTTP test with multiple clients
async fn run_http_test(
    handle: ReactorHandle,
    listener: TcpListener,
    num_clients: usize,
) -> Result<(), BoxError> {
    println!(
        "\n--- Running HTTP/1.1 test with {} clients ---\n",
        num_clients
    );

    // Create cancellation token for shutdown
    let cancel = CancellationToken::new();

    // Create and spawn HTTP/1.1 server
    let server = Http1Server::new(listener, cancel.clone(), echo_service, 0, SERVER_PORT);
    let server_handle = tokio::task::spawn_local(server.run());

    // Spawn client tasks
    let mut client_handles = Vec::with_capacity(num_clients);
    for i in 0..num_clients {
        let local_port = 49152 + i as u16;
        let handle_clone = handle.clone();

        let client_handle =
            tokio::task::spawn_local(
                async move { run_http_client(&handle_clone, i, local_port).await },
            );
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
    cancel.cancel();

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

    println!("\n✓ All {} HTTP clients verified!", num_clients);
    Ok(())
}

#[test]
#[serial]
fn test_http_echo() {
    const NUM_CLIENTS: usize = 3;

    println!("\n=== HTTP/1.1 Echo Test ===\n");

    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    println!("EAL initialized");

    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(Ipv4Address::new(192, 168, 1, 254))
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(|ctx: WorkerContext| async move {
            let listener = TcpListener::bind_with_backlog(
                &ctx.reactor,
                SERVER_PORT,
                8192,
                8192,
                NUM_CLIENTS + 1,
            )
            .expect("Failed to bind listener");

            let result = run_http_test(ctx.reactor.clone(), listener, NUM_CLIENTS).await;

            match result {
                Ok(()) => {
                    println!("\n--- Test Result ---");
                    println!(
                        "\n✓ HTTP/1.1 Echo Test PASSED ({} clients served)!\n",
                        NUM_CLIENTS
                    );
                }
                Err(e) => {
                    panic!("Test failed: {}", e);
                }
            }
        });

    println!("\n=== HTTP/1.1 Echo Test Complete ===\n");
}

//! TCP Echo Async Test
//!
//! This test creates an async TCP echo server and client using DPDK with smoltcp.
//! It demonstrates the tcp module's TcpListener and TcpStream APIs with tokio integration.
//!
//! The test spawns separate tokio tasks for:
//! - The reactor (polling DPDK)
//! - The server (accepting and echoing)
//! - Each client (connecting, sending, receiving)
//!
//! Note: This is a separate test file because DPDK has global state that persists
//! across tests within the same process.

use dpdk_net::BoxError;
use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::runtime::ReactorHandle;
use dpdk_net::socket::{TcpListener, TcpStream};
use dpdk_net_axum::{DpdkApp, WorkerContext};
use dpdk_net_test::app::echo_server::{EchoServer, ServerStats};
use serial_test::serial;
use smoltcp::wire::{IpAddress, Ipv4Address};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const SERVER_PORT: u16 = 8080;
const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const NUM_CLIENTS: usize = 5;

/// Run a single client task: connect, send, receive, verify
async fn run_client(
    handle: &ReactorHandle,
    client_id: usize,
    local_port: u16,
) -> Result<(), BoxError> {
    // Connect to server
    let client = TcpStream::connect(
        handle,
        IpAddress::Ipv4(SERVER_IP),
        SERVER_PORT,
        local_port,
        4096,
        4096,
    )
    .map_err(|e| format!("Client {}: connect failed: {:?}", client_id, e))?;

    println!("Client {}: created (local port {})", client_id, local_port);

    // Wait for connection to be established
    client
        .wait_connected()
        .await
        .map_err(|e| format!("Client {}: connection failed: {:?}", client_id, e))?;
    println!("Client {}: connected", client_id);

    // Send a message
    let message = format!("Hello from client {}!", client_id);
    client
        .send(message.as_bytes())
        .await
        .map_err(|e| format!("Client {}: send failed: {:?}", client_id, e))?;
    println!("Client {}: sent '{}'", client_id, message);

    // Receive echo
    let mut buf = [0u8; 1024];
    let len = client
        .recv(&mut buf)
        .await
        .map_err(|e| format!("Client {}: recv failed: {:?}", client_id, e))?;

    let received = std::str::from_utf8(&buf[..len])
        .map_err(|_| format!("Client {}: invalid utf8", client_id))?;

    // Verify echo
    if received != message {
        return Err(format!(
            "Client {}: MISMATCH! expected '{}', got '{}'",
            client_id, message, received
        )
        .into());
    }

    println!("Client {}: echo verified ✓", client_id);

    // Close gracefully
    client.close().await;

    Ok(())
}

// Using EchoServer from dpdk_net_test::app::echo_server

// Using handle_connection from dpdk_net_test::app::echo_server

/// Test N clients connecting simultaneously and being served
async fn run_multi_client_test(
    handle: ReactorHandle,
    listener: TcpListener,
    num_clients: usize,
) -> Result<(), BoxError> {
    println!(
        "\n--- Running multi-client test with {} clients ---\n",
        num_clients
    );

    // Create cancellation token for shutdown
    let cancel = CancellationToken::new();
    let stats = Arc::new(ServerStats::new());

    // Create and spawn EchoServer
    let server = EchoServer::new(listener, cancel.clone(), stats, 0, SERVER_PORT);
    let server_handle = tokio::task::spawn_local(server.run());

    // Spawn client tasks and collect their handles
    let mut client_handles = Vec::with_capacity(num_clients);
    for i in 0..num_clients {
        let local_port = 49152 + i as u16;
        let handle_clone = handle.clone();

        let client_handle =
            tokio::task::spawn_local(async move { run_client(&handle_clone, i, local_port).await });
        client_handles.push(client_handle);
    }

    // Wait for all clients to complete
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

    // Wait for server to finish
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

    println!("\n✓ All {} clients verified!", num_clients);
    Ok(())
}

#[test]
#[serial]
fn test_tcp_echo_async() {
    println!("\n=== TCP Echo Async Test ===\n");

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
                4096,
                4096,
                NUM_CLIENTS + 1,
            )
            .expect("Failed to bind listener");

            let result = run_multi_client_test(ctx.reactor.clone(), listener, NUM_CLIENTS).await;

            match result {
                Ok(()) => {
                    println!("\n--- Test Result ---");
                    println!(
                        "\n✓ TCP Echo Async Test PASSED ({} clients served)!\n",
                        NUM_CLIENTS
                    );
                }
                Err(e) => {
                    panic!("Test failed: {}", e);
                }
            }
        });

    println!("\n=== TCP Echo Async Test Complete ===\n");
}

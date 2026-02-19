//! DpdkApp Echo Test
//!
//! This test validates the DpdkApp API by creating a single-lcore application
//! that runs both a TCP listener (server) and a TCP client that performs an echo test.
//!
//! Note: This test uses a virtual ring device for loopback testing.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::{TcpListener, TcpStream};
use dpdk_net_axum::{DpdkApp, WorkerContext};

use smoltcp::wire::{IpAddress, Ipv4Address};

use serial_test::serial;

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const SERVER_PORT: u16 = 8080;
const CLIENT_PORT: u16 = 49152;

/// Echo server that accepts one connection, receives data, echoes it back, then signals done.
async fn run_echo_server(mut listener: TcpListener, done_tx: tokio::sync::oneshot::Sender<()>) {
    println!("Server: waiting for connection...");

    // Accept one connection
    let stream = match listener.accept().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Server: accept failed: {:?}", e);
            let _ = done_tx.send(());
            return;
        }
    };

    println!("Server: connection accepted");

    // Receive data
    let mut buf = [0u8; 1024];
    let len = match stream.recv(&mut buf).await {
        Ok(n) => n,
        Err(e) => {
            eprintln!("Server: recv failed: {:?}", e);
            let _ = done_tx.send(());
            return;
        }
    };

    let received = String::from_utf8_lossy(&buf[..len]);
    println!("Server: received '{}' ({} bytes)", received, len);

    // Echo back
    if let Err(e) = stream.send(&buf[..len]).await {
        eprintln!("Server: send failed: {:?}", e);
        let _ = done_tx.send(());
        return;
    }

    println!("Server: echoed back");

    // Wait a bit for client to receive, then close
    tokio::task::yield_now().await;
    stream.close().await;

    // Signal done
    let _ = done_tx.send(());
}

/// Echo client that connects, sends data, receives echo, and verifies.
async fn run_echo_client(
    ctx: &WorkerContext,
    done_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    println!("Client: connecting...");

    // Connect to server
    let stream = TcpStream::connect(
        &ctx.reactor,
        IpAddress::Ipv4(SERVER_IP),
        SERVER_PORT,
        CLIENT_PORT,
        4096,
        4096,
    )
    .map_err(|e| format!("Client: connect failed: {:?}", e))?;

    // Wait for connection
    stream
        .wait_connected()
        .await
        .map_err(|_| "Client: TCP connection failed")?;

    println!("Client: connected");

    // Send message
    let message = "Hello from DpdkApp test!";
    stream
        .send(message.as_bytes())
        .await
        .map_err(|e| format!("Client: send failed: {:?}", e))?;

    println!("Client: sent '{}'", message);

    // Receive echo
    let mut buf = [0u8; 1024];
    let len = stream
        .recv(&mut buf)
        .await
        .map_err(|e| format!("Client: recv failed: {:?}", e))?;

    let received = std::str::from_utf8(&buf[..len]).map_err(|_| "Client: invalid utf8")?;

    println!("Client: received '{}' ({} bytes)", received, len);

    // Verify
    if received != message {
        return Err(format!(
            "Client: MISMATCH! expected '{}', got '{}'",
            message, received
        ));
    }

    println!("Client: echo verified ✓");

    // Close gracefully
    stream.close().await;

    // Wait for server to finish
    let _ = done_rx.await;

    Ok(())
}

/// Main server logic that runs on the lcore.
async fn server_main(ctx: WorkerContext) {
    println!(
        "Worker starting on lcore {} (queue {})",
        ctx.lcore.id(),
        ctx.queue_id
    );

    // Create channels for coordination
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    // Create listener
    let listener =
        TcpListener::bind(&ctx.reactor, SERVER_PORT, 4096, 4096).expect("Failed to bind listener");

    println!("Server: listening on port {}", SERVER_PORT);

    // Spawn server task
    let server_task = tokio::task::spawn_local(run_echo_server(listener, done_tx));

    // Give server a moment to start listening
    tokio::task::yield_now().await;

    // Run client
    let client_result = run_echo_client(&ctx, done_rx).await;

    // Wait for server task
    let _ = server_task.await;

    // Check result
    match client_result {
        Ok(()) => println!("\n✓ Echo test PASSED!"),
        Err(e) => panic!("Echo test FAILED: {}", e),
    }
}

#[test]
#[serial]
fn test_dpdk_app_echo() {
    println!("\n=== DpdkApp Echo Test ===\n");

    // Initialize EAL with virtual device (no hugepages needed for ring device)
    // Use -l 0 to specify just lcore 0 (main lcore) for single-lcore test
    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    println!("EAL initialized");

    // Run the DpdkApp
    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(GATEWAY_IP)
        .mbufs_per_queue(1024) // Small for testing without hugepages
        .descriptors(128, 128)
        .run(server_main);

    println!("\n=== DpdkApp Echo Test Complete ===\n");
}

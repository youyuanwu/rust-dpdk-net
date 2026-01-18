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
use dpdk_net::tcp::{Reactor, ReactorHandle, TcpListener, TcpStream};
use dpdk_net_test::dpdk_test::DpdkTestContextBuilder;
use smoltcp::iface::{Config, Interface};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use std::future::Future;
use tokio::runtime::Builder;

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

/// Run the server task: accept connections and echo data until shutdown
///
/// AcceptFuture is cancel-safe, so we can use tokio::select! to race
/// accept against the shutdown signal.
async fn run_server<F: Future<Output = ()>>(mut listener: TcpListener, shutdown: F) {
    println!(
        "Server: listening on {}:{} (backlog={})",
        SERVER_IP,
        SERVER_PORT,
        listener.backlog()
    );

    // Pin the shutdown future so we can poll it repeatedly
    tokio::pin!(shutdown);

    let mut conn_id = 0usize;
    loop {
        tokio::select! {
            // Shutdown signal received
            _ = &mut shutdown => {
                println!("Server: shutdown signal received");
                break;
            }
            // New connection accepted
            result = listener.accept() => {
                match result {
                    Ok(stream) => {
                        let id = conn_id;
                        conn_id += 1;
                        println!("Server: accepted connection {}", id);

                        // Spawn handler as background task
                        tokio::task::spawn_local(async move {
                            handle_server_connection(stream, id).await;
                        });
                    }
                    Err(e) => {
                        eprintln!("Server: accept failed: {:?}", e);
                        break;
                    }
                }
            }
        }
    }
}

/// Handle a single server connection: receive, echo, and close
async fn handle_server_connection(stream: TcpStream, conn_id: usize) {
    let mut buf = [0u8; 1024];

    // Receive data
    let len = match stream.recv(&mut buf).await {
        Ok(len) => len,
        Err(e) => {
            eprintln!("Server {}: recv failed: {:?}", conn_id, e);
            return;
        }
    };

    let msg = std::str::from_utf8(&buf[..len]).unwrap_or("<invalid>");
    println!("Server {}: received '{}'", conn_id, msg);

    // Echo it back
    if let Err(e) = stream.send(&buf[..len]).await {
        eprintln!("Server {}: send failed: {:?}", conn_id, e);
        return;
    }

    println!("Server {}: echoed '{}'", conn_id, msg);

    // Close gracefully and wait for shutdown to complete
    stream.close().await;
}

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

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn server task with shutdown signal
    let server_handle = tokio::task::spawn_local(run_server(listener, async {
        let _ = shutdown_rx.await;
    }));

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
    let _ = shutdown_tx.send(());

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
fn test_tcp_echo_async() {
    println!("\n=== TCP Echo Async Test ===\n");

    // Create DPDK test context using the shared harness (with pool for Reactor)
    let (_ctx, mut device) = DpdkTestContextBuilder::new()
        .vdev("net_ring0")
        .mempool_name("async_test_pool")
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

    // Create single-threaded tokio runtime
    // We use current_thread because DPDK and smoltcp are not thread-safe
    let rt = Builder::new_current_thread().enable_all().build().unwrap();

    // Create a LocalSet to run !Send futures (Rc-based reactor)
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        // Create the async reactor
        let reactor = Reactor::new(device, iface);
        let handle = reactor.handle();

        // Spawn the reactor polling task (runs in background)
        tokio::task::spawn_local(async move {
            reactor.run().await;
        });

        // Create server listener with backlog = NUM_CLIENTS + 1 to handle burst
        let listener =
            TcpListener::bind_with_backlog(&handle, SERVER_PORT, 4096, 4096, NUM_CLIENTS + 1)
                .expect("Failed to bind listener");

        // Run the async test with separate tasks for clients and server
        let result = run_multi_client_test(handle, listener, NUM_CLIENTS).await;

        // Verify the result
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
}

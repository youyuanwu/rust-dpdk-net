//! Bridge TCP listener test.
//!
//! Tests that an OS thread can bind a listener through the DPDK bridge,
//! accept an incoming connection from an lcore-local client, and
//! exchange data over the proxied stream.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::TcpStream;
use dpdk_net_util::{DpdkApp, DpdkBridge, WorkerContext};
use serial_test::serial;
use smoltcp::wire::{IpAddress, Ipv4Address};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use tokio_util::compat::FuturesAsyncReadCompatExt;

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const LISTENER_PORT: u16 = 9091;

#[test]
#[serial]
fn test_bridge_listener_accept() {
    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    // Create bridge pair
    let (bridge, bridge_workers) = DpdkBridge::pair();
    let listener_ready = Arc::new(Notify::new());
    let done = Arc::new(Notify::new());

    // OS thread binds a listener via bridge, accepts one connection, echoes data
    let bridge_handle = bridge.clone();
    let listener_ready_clone = listener_ready.clone();
    let done_clone = done.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            bridge_handle.wait_ready().await;

            // Bind listener through the bridge
            let mut listener = bridge_handle
                .listen(LISTENER_PORT)
                .await
                .expect("bridge listen failed");

            // Signal that the listener is ready for connections
            listener_ready_clone.notify_one();

            // Accept one connection
            let stream = listener.accept().await.expect("accept failed");
            let mut stream = stream.compat();

            // Read data from the client
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.expect("read failed");

            // Echo it back
            stream.write_all(&buf[..n]).await.expect("write failed");

            // Keep the stream alive until the remote side closes.
            // Without this, dropping BridgeTcpStream closes the relay
            // channels and the lcore-side TcpStream is aborted (RST)
            // before the echoed data reaches the client.
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });
        done_clone.notify_one();
    });

    // Lcore: bridge worker + client that connects to the bridge listener
    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(Ipv4Address::new(192, 168, 1, 254))
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(move |ctx: WorkerContext| {
            let bridge_workers = bridge_workers.clone();
            let listener_ready = listener_ready.clone();
            let done = done.clone();
            async move {
                // Register bridge worker
                bridge_workers.spawn(&ctx.reactor);

                // Wait for the OS thread to bind the listener via bridge
                listener_ready.notified().await;

                // Connect from the lcore side as a regular DPDK client
                let client = TcpStream::connect(
                    &ctx.reactor,
                    IpAddress::Ipv4(SERVER_IP),
                    LISTENER_PORT,
                    50000,
                    4096,
                    4096,
                )
                .expect("client connect failed");

                client
                    .wait_connected()
                    .await
                    .expect("client handshake failed");

                // Send data
                let message = b"hello via bridge listener";
                client.send(message).await.expect("client send failed");

                // Read echoed response
                let mut buf = [0u8; 1024];
                let n = client.recv(&mut buf).await.expect("client recv failed");
                assert_eq!(&buf[..n], message, "echo mismatch");

                // Close gracefully — this sends FIN so the OS thread's
                // read-until-EOF loop terminates.
                client.close().await.ok();

                // Wait for OS thread to signal completion
                done.notified().await;
            }
        });
}

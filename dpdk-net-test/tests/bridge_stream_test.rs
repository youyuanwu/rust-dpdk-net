//! Bridge TCP stream connect test.
//!
//! Tests that an OS thread can connect through the DPDK bridge to
//! reach a TCP echo server running on an lcore, send data, and
//! receive it back correctly.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::TcpListener;
use dpdk_net_test::app::echo_server::{EchoServer, ServerStats};
use dpdk_net_util::{DpdkApp, DpdkBridge, WorkerContext};
use serial_test::serial;
use smoltcp::wire::{IpAddress, Ipv4Address};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::sync::CancellationToken;

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const SERVER_PORT: u16 = 9090;

#[test]
#[serial]
fn test_bridge_stream_echo() {
    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    // Create bridge pair before run() blocks
    let (bridge, bridge_workers) = DpdkBridge::pair();

    // Notify used to signal when the OS thread is done
    let done = Arc::new(Notify::new());

    // Spawn OS thread that will use the bridge
    let bridge_handle = bridge.clone();
    let done_clone = done.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Wait for the lcore worker to register
            bridge_handle.wait_ready().await;

            // Connect through the bridge
            let stream = bridge_handle
                .connect(IpAddress::Ipv4(SERVER_IP), SERVER_PORT)
                .await
                .expect("bridge connect failed");

            // Bridge to tokio traits via compat
            let mut stream = stream.compat();

            // Send a message
            let message = b"hello from bridge";
            stream.write_all(message).await.expect("write failed");

            // Read echo response
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.expect("read failed");
            let received = &buf[..n];

            assert_eq!(received, message, "echo mismatch");
        });
        done_clone.notify_one();
    });

    // Run DPDK app: echo server + bridge worker on the same lcore
    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(Ipv4Address::new(192, 168, 1, 254))
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(move |ctx: WorkerContext| {
            let bridge_workers = bridge_workers.clone();
            let done = done.clone();
            async move {
                // Register this lcore as a bridge worker
                bridge_workers.spawn(&ctx.reactor);

                // Start echo server
                let cancel = CancellationToken::new();
                let stats = Arc::new(ServerStats::new());
                let listener =
                    TcpListener::bind(&ctx.reactor, SERVER_PORT, 4096, 4096).expect("bind failed");
                let server = EchoServer::new(listener, cancel.clone(), stats, 0, SERVER_PORT);

                let server_handle = tokio::task::spawn_local(server.run());

                // Wait for the OS thread to signal completion, then shut down
                let cancel_clone = cancel.clone();
                let done = done.clone();
                tokio::task::spawn_local(async move {
                    done.notified().await;
                    cancel_clone.cancel();
                });

                server_handle.await.expect("server panicked");
            }
        });
}

//! Bridge UDP socket test.
//!
//! Tests that an OS thread can bind a UDP socket through the DPDK bridge,
//! send a datagram to a UDP echo server running on the lcore, and receive
//! the echoed datagram back.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::UdpSocket;
use dpdk_net_util::{DpdkApp, DpdkBridge, WorkerContext};
use serial_test::serial;
use smoltcp::wire::Ipv4Address;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const ECHO_PORT: u16 = 7070;
const BRIDGE_PORT: u16 = 6060;

/// UDP echo server: receives datagrams and sends them back until cancelled.
async fn udp_echo_loop(socket: UdpSocket, cancel: CancellationToken) {
    let mut buf = [0u8; 1500];
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            result = socket.recv_from(&mut buf) => {
                if let Ok((len, meta)) = result {
                    let _ = socket.send_to(&buf[..len], meta.endpoint).await;
                }
            }
        }
    }
}

#[test]
#[serial]
fn test_bridge_udp_echo() {
    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    let (bridge, bridge_workers) = DpdkBridge::pair();
    let echo_ready = Arc::new(Notify::new());
    let done = Arc::new(Notify::new());

    // OS thread: bind UDP via bridge, send datagram, receive echo
    let bridge_handle = bridge.clone();
    let echo_ready_clone = echo_ready.clone();
    let done_clone = done.clone();
    let os_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // catch_unwind so done is always notified (prevents lcore hang)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.block_on(async {
                bridge_handle.wait_ready().await;

                let socket = bridge_handle
                    .bind_udp(BRIDGE_PORT)
                    .await
                    .expect("bridge bind_udp failed");

                let local = socket.local_addr().expect("local_addr failed");
                assert_eq!(local.port(), BRIDGE_PORT);

                // Wait for the echo server to be ready on the lcore
                echo_ready_clone.notified().await;

                let target = SocketAddr::new("192.168.1.1".parse().unwrap(), ECHO_PORT);
                let message = b"hello from bridge udp";

                socket
                    .send_to(message, target)
                    .await
                    .expect("send_to failed");

                // Receive with timeout to avoid hanging on failure
                let mut buf = vec![0u8; 1500];
                let result =
                    tokio::time::timeout(Duration::from_secs(5), socket.recv_from(&mut buf)).await;

                match result {
                    Ok(Ok((n, from))) => {
                        assert_eq!(&buf[..n], message, "echo mismatch");
                        assert_eq!(from.port(), ECHO_PORT);
                    }
                    Ok(Err(e)) => panic!("recv_from error: {e}"),
                    Err(_) => panic!("timeout waiting for echo (5s)"),
                }
            });
        }));

        // Always notify done so lcore can shut down
        done_clone.notify_one();

        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    });

    // Lcore: bridge worker + UDP echo server
    DpdkApp::new()
        .eth_dev(0)
        .ip(SERVER_IP)
        .gateway(Ipv4Address::new(192, 168, 1, 254))
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(move |ctx: WorkerContext| {
            let bridge_workers = bridge_workers.clone();
            let echo_ready = echo_ready.clone();
            let done = done.clone();
            async move {
                bridge_workers.spawn(&ctx.reactor);

                let cancel = CancellationToken::new();

                let echo_socket = UdpSocket::bind(&ctx.reactor, ECHO_PORT, 64, 64, 1500)
                    .expect("echo bind failed");

                echo_ready.notify_one();

                let cancel_clone = cancel.clone();
                let echo_handle =
                    tokio::task::spawn_local(udp_echo_loop(echo_socket, cancel_clone));

                done.notified().await;
                cancel.cancel();
                echo_handle.await.expect("echo task panicked");
            }
        });

    os_thread.join().expect("OS thread panicked");
}

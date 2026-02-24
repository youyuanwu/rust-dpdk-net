//! UDP Echo Test using DpdkApp
//!
//! End-to-end test that verifies UDP send_to + recv_from works through the
//! full DPDK stack using the DpdkApp runner. Also exercises the new
//! `bind_default`, `connect`, `send`, and `recv` APIs.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::UdpSocket;
use dpdk_net_axum::{DpdkApp, WorkerContext};
use serial_test::serial;
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

const IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const SERVER_PORT: u16 = 9000;
const CLIENT_PORT: u16 = 9001;

#[test]
#[serial]
fn test_udp_echo_via_dpdk_app() {
    println!("\n=== UDP Echo DpdkApp Test ===\n");

    let _eal = EalBuilder::new()
        .no_huge()
        .no_pci()
        .in_memory()
        .core_list("0")
        .vdev("net_ring0")
        .init()
        .expect("Failed to initialize EAL");

    DpdkApp::new()
        .eth_dev(0)
        .ip(IP)
        .gateway(GATEWAY)
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(|ctx: WorkerContext| async move {
            // Test 1: Basic send_to / recv_from
            {
                let server = UdpSocket::bind_default(&ctx.reactor, SERVER_PORT)
                    .expect("Failed to bind server");
                let client = UdpSocket::bind_default(&ctx.reactor, CLIENT_PORT)
                    .expect("Failed to bind client");

                let server_ep = IpEndpoint::new(IpAddress::Ipv4(IP), SERVER_PORT);

                // Send from client to server
                let sent = client.send_to(b"hello udp", server_ep).await.unwrap();
                assert_eq!(sent, 9);
                println!("Client sent {} bytes", sent);

                // Yield to let reactor process
                tokio::task::yield_now().await;
                tokio::task::yield_now().await;

                // Server receives
                let mut buf = [0u8; 256];
                let (n, meta) = server.recv_from(&mut buf).await.unwrap();
                assert_eq!(&buf[..n], b"hello udp");
                println!("Server received: {:?}", std::str::from_utf8(&buf[..n]));

                // Server echoes back
                let sent = server.send_to(&buf[..n], meta.endpoint).await.unwrap();
                assert_eq!(sent, 9);

                // Yield to let reactor process
                tokio::task::yield_now().await;
                tokio::task::yield_now().await;

                // Client receives echo
                let (n2, _) = client.recv_from(&mut buf).await.unwrap();
                assert_eq!(&buf[..n2], b"hello udp");
                println!("Client received echo: {:?}", std::str::from_utf8(&buf[..n2]));
                println!("Test 1 (send_to/recv_from) PASSED");
            }

            // Test 2: Connected mode (connect + send/recv)
            {
                let mut server = UdpSocket::bind_default(&ctx.reactor, SERVER_PORT + 10)
                    .expect("Failed to bind server");
                let mut client = UdpSocket::bind_default(&ctx.reactor, CLIENT_PORT + 10)
                    .expect("Failed to bind client");

                let server_ep = IpEndpoint::new(IpAddress::Ipv4(IP), SERVER_PORT + 10);
                let client_ep = IpEndpoint::new(IpAddress::Ipv4(IP), CLIENT_PORT + 10);

                client.connect(server_ep);
                server.connect(client_ep);

                assert_eq!(client.peer_endpoint(), Some(server_ep));
                assert_eq!(server.peer_endpoint(), Some(client_ep));

                // Send via connected mode
                let sent = client.send(b"connected!").await.unwrap();
                assert_eq!(sent, 10);

                tokio::task::yield_now().await;
                tokio::task::yield_now().await;

                // Receive via connected mode (no metadata)
                let mut buf = [0u8; 256];
                let n = server.recv(&mut buf).await.unwrap();
                assert_eq!(&buf[..n], b"connected!");
                println!("Test 2 (connected mode) PASSED");
            }

            // Test 3: WorkerContext helpers
            {
                let socket = ctx.bind_udp(SERVER_PORT + 20).expect("Failed to bind via ctx");
                assert!(socket.is_open());
                assert_eq!(socket.endpoint().port, SERVER_PORT + 20);

                let socket2 = ctx.bind_udp_with_buffers(SERVER_PORT + 21, 32, 32, 1500)
                    .expect("Failed to bind via ctx with custom buffers");
                assert!(socket2.is_open());
                println!("Test 3 (WorkerContext helpers) PASSED");
            }

            // Test 4: Ephemeral port allocation
            {
                let port1 = ctx.alloc_ephemeral_port();
                let port2 = ctx.alloc_ephemeral_port();
                assert_ne!(port1, port2);
                assert!(port1 >= 32768);
                assert!(port2 >= 32768);
                println!("Test 4 (ephemeral ports: {}, {}) PASSED", port1, port2);
            }

            println!("\n=== All UDP DpdkApp Tests PASSED ===\n");
        });
}

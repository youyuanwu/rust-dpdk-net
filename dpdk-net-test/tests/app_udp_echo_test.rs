//! DpdkApp UDP Echo Test
//!
//! Validates the async UDP socket through DpdkApp. A "server" socket and a
//! "client" socket are both bound on the same lcore. The client sends a
//! datagram to the server, the server echoes it back, and the client
//! verifies the payload.
//!
//! Uses `net_ring0` for loopback: transmitted frames re-enter the RX path.

use dpdk_net::api::rte::eal::EalBuilder;
use dpdk_net::socket::UdpSocket;
use dpdk_net_util::{DpdkApp, WorkerContext};

use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

use serial_test::serial;

const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const GATEWAY_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 254);
const SERVER_PORT: u16 = 7777;
const CLIENT_PORT: u16 = 8888;

async fn udp_echo_main(ctx: WorkerContext) {
    println!(
        "Worker starting on lcore {} (queue {})",
        ctx.lcore.id(),
        ctx.queue_id
    );

    // Bind server and client sockets on the same reactor
    let server = UdpSocket::bind(&ctx.reactor, SERVER_PORT, 16, 16, 1500)
        .expect("Failed to bind server socket");
    let client = UdpSocket::bind(&ctx.reactor, CLIENT_PORT, 16, 16, 1500)
        .expect("Failed to bind client socket");

    println!("Server bound on port {SERVER_PORT}, client on port {CLIENT_PORT}");

    let message = b"Hello UDP from DpdkApp!";
    let server_endpoint = IpEndpoint::new(IpAddress::Ipv4(SERVER_IP), SERVER_PORT);

    // Client sends to server
    let sent = client
        .send_to(message, server_endpoint)
        .await
        .expect("Client: send_to failed");
    println!("Client: sent {sent} bytes");

    // Server receives the datagram
    let mut buf = [0u8; 1500];
    let (len, meta) = server
        .recv_from(&mut buf)
        .await
        .expect("Server: recv_from failed");
    println!("Server: received {} bytes from {:?}", len, meta.endpoint);
    assert_eq!(&buf[..len], message, "Server: payload mismatch");

    // Server echoes back to the client's endpoint
    server
        .send_to(&buf[..len], meta.endpoint)
        .await
        .expect("Server: echo send_to failed");
    println!("Server: echoed back");

    // Client receives the echo
    let (len, meta) = client
        .recv_from(&mut buf)
        .await
        .expect("Client: recv_from failed");
    println!("Client: received {} bytes from {:?}", len, meta.endpoint);
    assert_eq!(&buf[..len], message, "Client: echo payload mismatch");
    assert_eq!(meta.endpoint.port, SERVER_PORT);

    println!("\n✓ UDP echo test PASSED!");

    drop(client);
    drop(server);
}

#[test]
#[serial]
fn test_dpdk_app_udp_echo() {
    println!("\n=== DpdkApp UDP Echo Test ===\n");

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
        .gateway(GATEWAY_IP)
        .mbufs_per_queue(1024)
        .descriptors(128, 128)
        .run(udp_echo_main);

    println!("\n=== DpdkApp UDP Echo Test Complete ===\n");
}

//! TCP Echo Loopback Test
//!
//! This test creates a TCP echo server and client using DPDK with smoltcp.
//! Since DPDK virtual devices (net_ring) don't automatically loopback,
//! we use a single device with both server and client sockets in the same
//! smoltcp interface, which handles the "loopback" at the TCP/IP layer.
//!
//! Note: This is a separate test file because DPDK has global state that persists
//! across tests within the same process.

use dpdk_net::tcp::{DEFAULT_MBUF_DATA_ROOM_SIZE, DEFAULT_MBUF_HEADROOM, DpdkDeviceWithPool};
use dpdk_net_test::tcp_echo::{EchoClient, EchoServer, SocketConfig, run_echo_test};
use rpkt_dpdk::*;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use std::time::Duration;

const SERVER_PORT: u16 = 8080;
const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);
const TEST_MESSAGE: &[u8] = b"Hello, Echo Server!";

#[test]
fn test_tcp_echo_loopback() {
    println!("\n=== TCP Echo Loopback Test ===\n");

    // Initialize DPDK with virtual ring device
    DpdkOption::new()
        .args(["--no-huge", "--no-pci", "--vdev=net_ring0"])
        .init()
        .unwrap();

    // Create mempool
    service()
        .mempool_alloc(
            "echo_test_pool",
            8192,
            256,
            DEFAULT_MBUF_DATA_ROOM_SIZE as u16,
            0,
        )
        .unwrap();

    // Configure port with 1 queue pair
    let eth_conf = EthConf::new();
    let rxq_confs = vec![RxqConf::new(1024, 0, "echo_test_pool")];
    let txq_confs = vec![TxqConf::new(1024, 0)];

    service()
        .dev_configure_and_start(0, &eth_conf, &rxq_confs, &txq_confs)
        .unwrap();

    // Get queue and mempool
    let rxq = service().rx_queue(0, 0).unwrap();
    let txq = service().tx_queue(0, 0).unwrap();
    let mempool = service().mempool("echo_test_pool").unwrap();

    // Create DPDK device
    let mbuf_capacity = DEFAULT_MBUF_DATA_ROOM_SIZE - DEFAULT_MBUF_HEADROOM;
    let mut device = DpdkDeviceWithPool::new(rxq, txq, mempool, 1500, mbuf_capacity);

    // Configure smoltcp interface
    let mac_addr = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    let config = Config::new(mac_addr.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());

    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(IpCidr::new(IpAddress::Ipv4(SERVER_IP), 24))
            .unwrap();
    });

    // Create socket set
    let mut sockets = SocketSet::new(vec![]);

    // Create server and client using the new API
    let mut server = EchoServer::new(&mut sockets, SERVER_PORT, SocketConfig::default());
    println!("Server listening on {}:{}", SERVER_IP, SERVER_PORT);

    let mut client = EchoClient::new(
        &mut sockets,
        &mut iface,
        SERVER_IP,
        SERVER_PORT,
        49152,
        SocketConfig::default(),
    );
    client.send(TEST_MESSAGE);
    println!("Client connecting to {}:{}", SERVER_IP, SERVER_PORT);

    // Run the test
    let result = run_echo_test(
        &mut device,
        &mut iface,
        &mut sockets,
        &mut server,
        &mut client,
        Duration::from_secs(5),
    );

    // Verify results
    println!("\n=== Test Results ===");
    println!("  Connected: {}", result.connected);
    println!("  Bytes sent: {}", result.bytes_sent);
    println!("  Bytes received: {}", result.bytes_received);
    println!("  Echo verified: {}", result.echo_verified);
    println!("  Server stats: {:?}", server.stats());

    assert!(result.connected, "Client should have connected");
    assert!(result.echo_verified, "Echoed data should match");
    assert_eq!(result.bytes_sent, TEST_MESSAGE.len());
    assert_eq!(result.bytes_received, TEST_MESSAGE.len());

    // Cleanup
    drop(sockets);
    drop(iface);
    drop(device);

    service().dev_stop_and_close(0).unwrap();
    service().mempool_free("echo_test_pool").unwrap();
    service().graceful_cleanup().unwrap();

    println!("\n=== Test Passed! ===\n");
}

//! TCP Echo Stress Test
//!
//! Tests the server's ability to handle multiple sequential connections.
//! Each client connects, exchanges messages, then closes before the next client starts.
//!
//! Note: This is a separate test file because DPDK has global state that persists
//! across tests within the same process.

use dpdk_net_test::dpdk_test::DpdkTestContextBuilder;
use dpdk_net_test::tcp_echo::{EchoServer, SocketConfig, run_stress_test};
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use std::time::Duration;

const SERVER_PORT: u16 = 8080;
const SERVER_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 1);

#[test]
fn test_tcp_echo_stress() {
    println!("\n=== TCP Echo Stress Test (Sequential Clients) ===\n");

    const NUM_ROUNDS: usize = 3;
    const MESSAGES_PER_ROUND: usize = 5;

    // Create DPDK test context using the shared harness
    let (_ctx, mut device) = DpdkTestContextBuilder::new()
        .vdev("net_ring0")
        .mempool_name("stress_test_pool")
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

    let mut sockets = SocketSet::new(vec![]);

    // Create server using the new API
    let mut server = EchoServer::new(&mut sockets, SERVER_PORT, SocketConfig::large());
    println!("[Server] Listening on {}:{}", SERVER_IP, SERVER_PORT);

    // Run the stress test
    let (all_passed, results) = run_stress_test(
        &mut device,
        &mut iface,
        &mut sockets,
        &mut server,
        SERVER_IP,
        NUM_ROUNDS,
        MESSAGES_PER_ROUND,
        Duration::from_secs(5),
    );

    // Print summary
    println!("\n=== Results ===");
    println!("Rounds: {}", NUM_ROUNDS);
    println!("Messages per round: {}", MESSAGES_PER_ROUND);
    println!("Server stats: {:?}", server.stats());

    let total_bytes: usize = results.iter().map(|r| r.bytes_sent).sum();
    println!("Total bytes echoed: {}", total_bytes);

    for (i, result) in results.iter().enumerate() {
        println!(
            "  Round {}: {} bytes sent, {} bytes received, verified: {}",
            i, result.bytes_sent, result.bytes_received, result.echo_verified
        );
    }

    // Cleanup happens automatically when _ctx is dropped

    assert!(all_passed, "Not all rounds completed successfully");
    println!("\n=== Stress Test Passed! ===\n");
}

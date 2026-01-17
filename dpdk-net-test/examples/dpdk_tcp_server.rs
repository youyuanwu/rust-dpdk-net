//! DPDK TCP Echo Server using smoltcp
//!
//! This example starts a TCP server on eth1 using DPDK+smoltcp.
//! It listens on port 8080 and echoes back any data received.
//!
//! Usage:
//!   sudo -E cargo run --example dpdk_tcp_server
//!
//! Then from another machine on the same network:
//!   nc 10.0.0.5 8080
//!   # Type messages and see them echoed back

use dpdk_net::tcp::{DEFAULT_MBUF_DATA_ROOM_SIZE, DEFAULT_MBUF_HEADROOM, DpdkDeviceWithPool};
use dpdk_net_test::echo_server::{EchoServerConfig, run_echo_server, setup_ctrlc_handler};
use rpkt_dpdk::*;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};

fn main() {
    // Setup hugepages
    dpdk_net_test::util::ensure_hugepages().unwrap();

    // Get network configuration for eth1
    let interface = "eth1";
    let ip_addr = dpdk_net_test::tcp::get_interface_ipv4(interface)
        .expect("Failed to get IP address for eth1");
    let gateway =
        dpdk_net_test::tcp::get_default_gateway().unwrap_or(Ipv4Address::new(10, 0, 0, 1));

    println!("\n========================================");
    println!("DPDK TCP Echo Server");
    println!("========================================");
    println!("IP Address: {}:8080", ip_addr);
    println!("Gateway: {}", gateway);
    println!("\nServer is starting...\n");

    // Get PCI address for eth1
    let pci_addr =
        dpdk_net_test::tcp::get_pci_addr(interface).expect("Failed to get PCI address for eth1");
    let args = format!("-a {}", pci_addr);

    // Initialize DPDK
    DpdkOption::new()
        .args(args.split(" ").collect::<Vec<_>>())
        .init()
        .unwrap();

    // Create mempool
    service()
        .mempool_alloc(
            "server_pool",
            8192,
            256,
            DEFAULT_MBUF_DATA_ROOM_SIZE as u16,
            0,
        )
        .unwrap();

    // Configure port
    let eth_conf = EthConf::new();
    let rxq_confs = vec![RxqConf::new(1024, 0, "server_pool")];
    let txq_confs = vec![TxqConf::new(1024, 0)];

    service()
        .dev_configure_and_start(0, &eth_conf, &rxq_confs, &txq_confs)
        .unwrap();

    // Get queues and mempool
    let rxq = service().rx_queue(0, 0).unwrap();
    let txq = service().tx_queue(0, 0).unwrap();
    let mempool = service().mempool("server_pool").unwrap();

    // Create DPDK device for smoltcp
    let mbuf_capacity = DEFAULT_MBUF_DATA_ROOM_SIZE - DEFAULT_MBUF_HEADROOM;
    let mut device = DpdkDeviceWithPool::new(rxq, txq, mempool, 1500, mbuf_capacity);

    // Get MAC address from DPDK
    let dev_info = service().dev_info(0).unwrap();
    let mac_addr = EthernetAddress(dev_info.mac_addr);

    // Configure smoltcp interface
    let config = Config::new(mac_addr.into());
    let mut iface = Interface::new(config, &mut device, Instant::now());

    // Set IP address
    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(IpCidr::new(IpAddress::Ipv4(ip_addr), 24))
            .unwrap();
    });

    // Add default route
    iface.routes_mut().add_default_ipv4_route(gateway).unwrap();

    println!("Interface configured:");
    println!("  IP: {}/24", ip_addr);
    println!("  MAC: {:?}", mac_addr);
    println!("  Gateway: {}", gateway);

    // Create socket set
    let mut sockets = SocketSet::new(vec![]);

    let port = 8080;
    println!("\n✓ Server will listen on {}:{}", ip_addr, port);
    println!("\nConnect from another machine:");
    println!("  nc {} {}", ip_addr, port);
    println!("\nPress Ctrl+C to stop the server\n");
    println!("========================================\n");

    // Setup Ctrl+C handler
    let running = setup_ctrlc_handler();

    // Run the echo server loop (creates and manages the socket internally)
    let result = run_echo_server(
        &mut device,
        &mut iface,
        &mut sockets,
        port,
        running,
        EchoServerConfig::default(),
    );

    // Print final statistics
    println!("\n========================================");
    println!("Server Statistics:");
    println!("  Runtime: {} seconds", result.runtime_secs);
    println!("  Connections: {}", result.stats.connections);
    println!("  Bytes received: {}", result.stats.bytes_received);
    println!("  Bytes sent: {}", result.stats.bytes_sent);
    println!("  Bytes dropped: {}", result.stats.bytes_dropped);
    println!("  Send errors: {}", result.stats.send_errors);
    println!("========================================\n");

    // Cleanup
    println!("Cleaning up...");
    drop(device);
    drop(sockets);
    drop(iface);

    service().dev_stop_and_close(0).unwrap();
    service().mempool_free("server_pool").unwrap();
    service().graceful_cleanup().unwrap();

    println!("Server stopped.");
}
